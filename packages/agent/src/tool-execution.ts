/**
 * @file 工具调度的并发、审批、超时与结果归一化。
 * @description 工具失败被转换为模型可见的 `isError` 结果；仅实现契约错误会向上抛出。
 * @module agent/tool-execution
 */

import { isAborted } from './abort.js';
import type { AgentSettings } from './agent-types.js';
import { AgentDefinitionError, AgentModelError, AgentToolError } from './errors.js';
import type { AgentEvent } from './events.js';
import { freezeAgentEvent } from './freeze.js';
import type { AgentMessage, ToolCallContentPart } from './message.js';
import { normalizeToolResult, type ToolDefinition, type ToolResult } from './tool.js';

/** 工具结果截断标记。 */
const TRUNCATION_MARKER = '\n...[truncated]';

/** `executeToolCalls` 的输入。 */
export interface ExecuteToolCallsOptions {
  /** 本轮模型请求的工具调用。 */
  readonly calls: readonly ToolCallContentPart[];
  /** 解析后的 Agent 设置。 */
  readonly settings: AgentSettings;
  /** 包含 assistant 消息的对话快照。 */
  readonly messages: readonly AgentMessage[];
  /** 取消信号。 */
  readonly signal: AbortSignal | undefined;
  /** 当前轮次编号。 */
  readonly step: number;
}

/** 单次工具调用所需上下文。 */
interface ExecuteToolCallOptions {
  /** 工具调用块。 */
  readonly call: ToolCallContentPart;
  /** 解析后的 Agent 设置。 */
  readonly settings: AgentSettings;
  /** 对话快照。 */
  readonly messages: readonly AgentMessage[];
  /** 取消信号。 */
  readonly signal: AbortSignal | undefined;
}

/** 工具执行结果分类。 */
type ToolRunOutcome =
  | { readonly kind: 'done'; readonly value: unknown }
  | { readonly kind: 'aborted' }
  | { readonly kind: 'timeout' };

/** 永不 resolve 的占位 Promise，表示未配置超时。 */
const NEVER: Promise<never> = new Promise<never>(() => {
  // 故意保持 pending：无超时场景不产生任何定时器或回调。
});

/**
 * 并发执行一轮工具调用，并按完成顺序发出事件。
 *
 * @param options - 调用列表、设置、对话快照与取消信号
 * @returns 事件流，返回值为与 `calls` 顺序一致的结果数组
 * @throws {AgentModelError} 同一响应出现重复 callId
 * @throws {AgentToolError} 工具实现返回非法结果或内部调度失败
 */
export async function* executeToolCalls(
  options: ExecuteToolCallsOptions,
): AsyncGenerator<AgentEvent, readonly ToolResult[], void> {
  const { calls, settings, messages, signal, step } = options;
  if (!Array.isArray(calls)) {
    throw new AgentDefinitionError('calls 必须是工具调用数组');
  }
  assertUniqueCallIds(calls, settings.providerId);
  if (calls.length === 0) return Object.freeze([]);

  const results: Array<ToolResult | undefined> = Array.from(
    { length: calls.length },
    () => undefined,
  );
  let running: Array<{ readonly index: number; readonly promise: Promise<SettledToolCall> }> = [];
  let nextIndex = 0;

  while (nextIndex < calls.length || running.length > 0) {
    const starting: Array<{ readonly index: number; readonly call: ToolCallContentPart }> = [];
    while (
      running.length + starting.length < settings.toolConcurrency &&
      nextIndex < calls.length
    ) {
      const index = nextIndex;
      nextIndex += 1;
      starting.push({ index, call: requireCall(calls, index) });
    }
    for (const entry of starting) {
      yield freezeAgentEvent({
        type: 'tool_start',
        step,
        callId: entry.call.callId,
        name: entry.call.name,
        input: entry.call.input,
      });
    }
    for (const entry of starting) {
      const promise = executeToolCall({
        call: entry.call,
        settings,
        messages,
        signal,
      }).then((result) => ({ index: entry.index, result }));
      running.push({ index: entry.index, promise });
    }

    const settled = await Promise.race(running.map((entry) => entry.promise));
    const position = running.findIndex((entry) => entry.index === settled.index);
    if (position < 0) {
      throw new AgentToolError(
        requireCall(calls, settled.index).name,
        '执行调度丢失了运行中的调用',
      );
    }
    running.splice(position, 1);
    results[settled.index] = settled.result;
    const call = requireCall(calls, settled.index);
    yield freezeAgentEvent({
      type: 'tool_end',
      step,
      callId: call.callId,
      name: call.name,
      result: settled.result,
    });
  }

  return Object.freeze(
    results.map((result, index) => {
      if (result === undefined) {
        throw new AgentToolError(requireCall(calls, index).name, '缺少执行结果');
      }
      return result;
    }),
  );
}

/** 已完成工具调用。 */
interface SettledToolCall {
  /** 调用在输入数组中的下标。 */
  readonly index: number;
  /** 归一化结果。 */
  readonly result: ToolResult;
}

/** 执行单个工具调用，所有预期失败都转换为结果。 */
async function executeToolCall(options: ExecuteToolCallOptions): Promise<ToolResult> {
  const { call, settings, messages, signal } = options;
  const maxLength = settings.maxToolResultLength;
  if (isAborted(signal)) return errorResult('工具调用已取消', maxLength);

  const tool = settings.registry.get(call.name);
  if (tool === undefined) {
    return errorResult(`未找到工具 "${call.name}"`, maxLength);
  }

  const approved = await requestApproval(settings, call, messages, signal);
  if (approved === undefined) return errorResult('工具调用已取消', maxLength);
  if (!approved) return errorResult('工具调用未获批准', maxLength);
  if (isAborted(signal)) return errorResult('工具调用已取消', maxLength);

  const parsed = parseInput(tool, call.input);
  if (!parsed.ok) return errorResult(`工具参数无效: ${parsed.message}`, maxLength);

  const timeoutMs = tool.timeoutMs ?? settings.toolTimeoutMs;
  let outcome: ToolRunOutcome;
  try {
    outcome = await runTool(tool, parsed.value, call, messages, signal, timeoutMs);
  } catch (error) {
    return errorResult(`工具执行失败: ${errorMessage(error)}`, maxLength);
  }

  if (outcome.kind === 'aborted') return errorResult('工具调用已取消', maxLength);
  if (outcome.kind === 'timeout') {
    return errorResult(`工具执行超时（>${String(timeoutMs)}ms）`, maxLength);
  }
  return finalizeResult(normalizeToolResult(outcome.value, call.name), maxLength);
}

/** 请求工具审批；undefined 表示运行被取消。 */
async function requestApproval(
  settings: AgentSettings,
  call: ToolCallContentPart,
  messages: readonly AgentMessage[],
  signal: AbortSignal | undefined,
): Promise<boolean | undefined> {
  const callback = settings.approveToolCall;
  if (callback === undefined) return true;
  if (isAborted(signal)) return undefined;

  let decision: boolean | Promise<boolean>;
  try {
    decision = callback(call, { messages, signal });
  } catch (error) {
    throw new AgentToolError(call.name, '审批回调失败', error);
  }

  const abortWaiter = createAbortWaiter(signal);
  try {
    let settled:
      { readonly kind: 'decision'; readonly value: unknown } | { readonly kind: 'aborted' };
    try {
      settled = await Promise.race([
        Promise.resolve(decision).then((value) => ({ kind: 'decision' as const, value })),
        abortWaiter.promise.then(() => ({ kind: 'aborted' as const })),
      ]);
    } catch (error) {
      throw new AgentToolError(call.name, '审批回调失败', error);
    }
    if (settled.kind === 'aborted') return undefined;
    if (isAborted(signal)) return undefined;
    if (typeof settled.value !== 'boolean') {
      throw new AgentToolError(call.name, '审批回调必须返回布尔值');
    }
    return settled.value;
  } finally {
    abortWaiter.cleanup();
  }
}

/** 解析工具输入；抛错时返回可回传模型的错误消息。 */
function parseInput(
  tool: ToolDefinition<unknown>,
  input: ToolCallContentPart['input'],
):
  | { readonly ok: true; readonly value: unknown }
  | { readonly ok: false; readonly message: string } {
  if (tool.parseInput === undefined) return { ok: true, value: input };
  try {
    return { ok: true, value: tool.parseInput(input) };
  } catch (error) {
    return { ok: false, message: errorMessage(error) };
  }
}

/** 执行工具并与取消/超时竞速。 */
async function runTool(
  tool: ToolDefinition<unknown>,
  input: unknown,
  call: ToolCallContentPart,
  messages: readonly AgentMessage[],
  signal: AbortSignal | undefined,
  timeoutMs: number | undefined,
): Promise<ToolRunOutcome> {
  if (isAborted(signal)) return { kind: 'aborted' };
  const controller = new AbortController();
  const abortWaiter = createAbortWaiter(signal);
  let timer: ReturnType<typeof setTimeout> | undefined;
  let cancellation: 'aborted' | 'timeout' | undefined;

  try {
    const execution = Promise.resolve(
      tool.execute(input, {
        callId: call.callId,
        name: call.name,
        signal: controller.signal,
        messages,
      }),
    ).then((value): ToolRunOutcome => ({ kind: 'done', value }));

    const abortOutcome = abortWaiter.promise.then((): ToolRunOutcome => {
      cancellation ??= 'aborted';
      controller.abort();
      return { kind: 'aborted' };
    });
    const timeoutOutcome =
      timeoutMs === undefined
        ? NEVER
        : new Promise<ToolRunOutcome>((resolve) => {
            // 仅用 Promise 包装 setTimeout 回调，便于与工具执行竞速。
            timer = setTimeout(() => {
              cancellation ??= 'timeout';
              controller.abort();
              resolve({ kind: 'timeout' });
            }, timeoutMs);
          });

    const outcome = await Promise.race([execution, abortOutcome, timeoutOutcome]);
    // 工具可能在收到 abort 后立即正常返回；取消/超时一旦触发必须优先。
    if (outcome.kind === 'done' && cancellation !== undefined) return { kind: cancellation };
    return outcome;
  } finally {
    abortWaiter.cleanup();
    if (timer !== undefined) clearTimeout(timer);
  }
}

/** 创建可清理的 abort 等待器。 */
function createAbortWaiter(signal: AbortSignal | undefined): {
  readonly promise: Promise<void>;
  cleanup(): void;
} {
  if (signal === undefined) return { promise: NEVER, cleanup: () => undefined };
  if (signal.aborted) return { promise: Promise.resolve(), cleanup: () => undefined };

  let listener: (() => void) | undefined;
  // 仅用 Promise 包装 AbortSignal 事件，便于与异步审批/工具执行竞速。
  const promise = new Promise<void>((resolve) => {
    listener = () => resolve();
    signal.addEventListener('abort', listener, { once: true });
  });
  return {
    promise,
    cleanup: (): void => {
      if (listener !== undefined) signal.removeEventListener('abort', listener);
    },
  };
}

/** 截断超长结果并追加标记。 */
function finalizeResult(result: ToolResult, maxLength: number): ToolResult {
  if (result.content.length <= maxLength) return result;
  // 标记本身也占用长度预算，保证最终 content.length 不超过 maxLength。
  if (maxLength <= TRUNCATION_MARKER.length) {
    return Object.freeze({ ...result, content: result.content.slice(0, maxLength) });
  }
  return Object.freeze({
    ...result,
    content: `${result.content.slice(0, maxLength - TRUNCATION_MARKER.length)}${TRUNCATION_MARKER}`,
  });
}

/** 构造错误结果。 */
function errorResult(message: string, maxLength: number): ToolResult {
  return finalizeResult(Object.freeze({ content: message, isError: true }), maxLength);
}

/** 校验调用 ID 在单次响应中唯一。 */
function assertUniqueCallIds(calls: readonly ToolCallContentPart[], providerId: string): void {
  const seen = new Set<string>();
  for (const call of calls) {
    if (seen.has(call.callId)) {
      throw new AgentModelError(providerId, `返回了重复的 toolCallId "${call.callId}"`);
    }
    seen.add(call.callId);
  }
}

/** 读取必定存在的调用。 */
function requireCall(calls: readonly ToolCallContentPart[], index: number): ToolCallContentPart {
  const call = calls[index];
  if (call === undefined) {
    throw new AgentToolError('internal', '工具调用下标越界');
  }
  return call;
}

/** 安全提取错误消息。 */
function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  try {
    return String(error);
  } catch {
    return '[unknown error]';
  }
}
