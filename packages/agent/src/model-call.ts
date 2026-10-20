/**
 * @file 模型调用：流式/非流式切换、响应校验、重试与事件输出。
 * @description 不写日志；所有观测信息通过事件流返回。
 * @module agent/model-call
 */

import { serializeError } from '@kurumi/core';

import { isAborted } from './abort.js';
import type { AgentSettings, PreparedRun } from './agent-types.js';
import { AgentModelError } from './errors.js';
import type { AgentEvent } from './events.js';
import { freezeAgentEvent } from './freeze.js';
import type { AgentMessage } from './message.js';
import {
  normalizeModelResponse,
  normalizeModelStreamEvent,
  type ModelCallOptions,
  type ModelRequest,
  type ResolvedModelResponse,
} from './model.js';
import { EMPTY_TOKEN_USAGE } from './usage.js';

/** `callModel` 的输入。 */
export interface CallModelOptions {
  /** 解析后的 Agent 设置。 */
  readonly settings: AgentSettings;
  /** 单次运行设置。 */
  readonly prepared: PreparedRun;
  /** 当前对话快照。 */
  readonly messages: readonly AgentMessage[];
  /** 当前轮次编号。 */
  readonly step: number;
}

/** 模型调用结果；`response` 为 undefined 表示运行被取消。 */
export interface ModelCallOutcome {
  /** 已校验的响应，或取消标记。 */
  readonly response: ResolvedModelResponse | undefined;
  /** 本次调用发生的重试次数。 */
  readonly retryCount: number;
}

/**
 * 调用模型并处理重试。
 *
 * @param options - 调用上下文
 * @returns 事件流，返回值为模型响应或取消标记
 */
export async function* callModel(
  options: CallModelOptions,
): AsyncGenerator<AgentEvent, ModelCallOutcome, void> {
  const { settings, prepared, step } = options;
  const request = createModelRequest(options);
  let retryCount = 0;
  let retryAttempt = 0;

  while (true) {
    try {
      const response = yield* invokeOnce(options, request);
      return { response, retryCount };
    } catch (error) {
      if (isAborted(prepared.signal)) return { response: undefined, retryCount };
      const normalized =
        error instanceof AgentModelError
          ? error
          : new AgentModelError(settings.providerId, '调用失败', error);
      if (
        retryAttempt >= settings.maxModelRetries ||
        !shouldRetry(settings, normalized, retryAttempt + 1)
      ) {
        throw normalized;
      }
      retryCount += 1;
      retryAttempt += 1;
      yield freezeAgentEvent({
        type: 'model_retry',
        step,
        attempt: retryCount,
        delayMs: settings.retryDelayMs,
        error: serializeError(normalized),
      });
      const shouldContinue = await waitBeforeRetry(settings.retryDelayMs, prepared.signal);
      if (!shouldContinue) return { response: undefined, retryCount };
    }
  }
}

/** 执行一次模型调用，优先使用流式能力。 */
async function* invokeOnce(
  options: CallModelOptions,
  request: ModelRequest,
): AsyncGenerator<AgentEvent, ResolvedModelResponse | undefined, void> {
  const { settings, prepared } = options;
  if (prepared.shouldStream && settings.provider.stream !== undefined) {
    return yield* invokeStream(options, request);
  }
  const response = await settings.provider.generate(request, toCallOptions(prepared.signal));
  if (isAborted(prepared.signal)) return undefined;
  return normalizeModelResponse(settings.providerId, response);
}

/** 消费流式响应并转发增量。 */
async function* invokeStream(
  options: CallModelOptions,
  request: ModelRequest,
): AsyncGenerator<AgentEvent, ResolvedModelResponse | undefined, void> {
  const { settings, prepared, step } = options;
  const stream = settings.provider.stream;
  if (stream === undefined) {
    throw new AgentModelError(settings.providerId, '未实现流式能力');
  }
  const iterator = stream
    .call(settings.provider, request, toCallOptions(prepared.signal))
    [Symbol.asyncIterator]();

  try {
    while (true) {
      const next = await iterator.next();
      if (next.done === true) {
        throw new AgentModelError(settings.providerId, '流在返回 response 事件前结束');
      }
      if (isAborted(prepared.signal)) return undefined;
      const event = normalizeModelStreamEvent(settings.providerId, next.value);
      if (event.type === 'response') {
        return {
          message: event.response.message,
          stopReason: event.response.stopReason,
          usage: event.response.usage ?? EMPTY_TOKEN_USAGE,
        };
      }
      yield freezeAgentEvent({ type: 'model_delta', step, delta: event });
    }
  } finally {
    await iterator.return?.();
  }
}

/** 构造冻结的模型请求。 */
function createModelRequest(options: CallModelOptions): ModelRequest {
  const { settings, prepared, messages } = options;
  return {
    model: settings.model,
    messages: Object.freeze([...messages]),
    tools: settings.registry.descriptors,
    ...(prepared.toolChoice === undefined ? {} : { toolChoice: prepared.toolChoice }),
    ...(settings.temperature === undefined ? {} : { temperature: settings.temperature }),
    ...(settings.maxOutputTokens === undefined
      ? {}
      : { maxOutputTokens: settings.maxOutputTokens }),
    ...(prepared.thinking === undefined ? {} : { thinking: prepared.thinking }),
    ...(settings.providerOptions === undefined
      ? {}
      : { providerOptions: settings.providerOptions }),
  };
}

/** 构造调用选项。 */
function toCallOptions(signal: AbortSignal | undefined): ModelCallOptions {
  return signal === undefined ? {} : { signal };
}

/** 执行重试判定回调。 */
function shouldRetry(settings: AgentSettings, error: unknown, attempt: number): boolean {
  let decision: unknown;
  try {
    decision = settings.shouldRetryModelError(error, attempt);
  } catch (cause) {
    throw new AgentModelError(settings.providerId, '重试判定回调失败', cause);
  }
  if (typeof decision !== 'boolean') {
    throw new AgentModelError(settings.providerId, '重试判定回调必须返回布尔值');
  }
  return decision;
}

/**
 * 等待重试间隔，并响应取消。
 *
 * @param delayMs - 等待毫秒数，非负安全整数
 * @param signal - 取消信号
 * @returns 正常等待完成返回 true；被取消返回 false
 */
function waitBeforeRetry(delayMs: number, signal: AbortSignal | undefined): Promise<boolean> {
  if (isAborted(signal)) return Promise.resolve(false);
  if (delayMs === 0) return Promise.resolve(true);

  // 仅用 Promise 包装 setTimeout/AbortSignal 回调，便于与 await 组合。
  return new Promise<boolean>((resolve) => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const onAbort = (): void => {
      if (timer !== undefined) clearTimeout(timer);
      signal?.removeEventListener('abort', onAbort);
      resolve(false);
    };
    timer = setTimeout(() => {
      signal?.removeEventListener('abort', onAbort);
      resolve(true);
    }, delayMs);
    signal?.addEventListener('abort', onAbort, { once: true });
  });
}
