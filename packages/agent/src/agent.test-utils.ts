/**
 * @file Agent 测试辅助。
 * @description 提供脚本化模型、消息响应、事件收集与取消等待器；不参与构建与覆盖率。
 * @module agent/test-utils
 */

import type { JsonValue } from '@kurumi/core';

import {
  ModelStopReason,
  createAssistantMessage,
  createTokenUsage,
  type AgentContentPart,
  type AgentEvent,
  type AgentResult,
  type ModelProvider,
  type ModelRequest,
  type ModelResponse,
  type ModelStreamEvent,
  type TokenUsage,
  type ToolCallContentPart,
} from './index.js';

/** 脚本化模型响应。 */
export type ScriptedResponse =
  | ModelResponse
  | ((request: ModelRequest, callIndex: number) => ModelResponse | Promise<ModelResponse>);

/** 脚本化模型提供方。 */
export interface ScriptedProvider {
  /** 提供方实例。 */
  readonly provider: ModelProvider;
  /** 每次调用收到的请求快照。 */
  readonly requests: ModelRequest[];
}

/**
 * 创建按顺序返回脚本响应的提供方。
 *
 * @param responses - 响应或响应工厂列表
 * @returns 提供方与请求记录
 */
export function createScriptedProvider(responses: readonly ScriptedResponse[]): ScriptedProvider {
  const requests: ModelRequest[] = [];
  let index = 0;
  return {
    requests,
    provider: {
      id: 'scripted',
      async generate(request): Promise<ModelResponse> {
        requests.push(request);
        const scripted = responses[index];
        index += 1;
        if (scripted === undefined) {
          throw new Error(`未准备第 ${String(index)} 个脚本响应`);
        }
        return typeof scripted === 'function' ? scripted(request, index - 1) : scripted;
      },
    },
  };
}

/**
 * 构造文本 assistant 响应。
 *
 * @param text - 文本内容
 * @param stopReason - 停止原因，缺省 end
 * @param usage - 可选用量
 * @returns 模型响应
 */
export function textResponse(
  text: string,
  stopReason: ModelStopReason = ModelStopReason.End,
  usage?: TokenUsage,
): ModelResponse {
  return {
    message: createAssistantMessage(text),
    stopReason,
    ...(usage === undefined ? {} : { usage }),
  };
}

/**
 * 构造工具调用 assistant 响应。
 *
 * @param calls - 工具调用块
 * @param usage - 可选用量
 * @returns 模型响应
 */
export function toolCallResponse(
  calls: readonly ToolCallContentPart[],
  usage?: TokenUsage,
): ModelResponse {
  return {
    message: createAssistantMessage(calls),
    stopReason: ModelStopReason.ToolCalls,
    ...(usage === undefined ? {} : { usage }),
  };
}

/**
 * 构造工具调用块。
 *
 * @param callId - 调用 ID
 * @param name - 工具名
 * @param input - JSON 输入
 * @returns 工具调用块
 */
export function toolCall(callId: string, name: string, input: JsonValue): ToolCallContentPart {
  return { type: 'tool_call', callId, name, input };
}

/**
 * 构造用量对象。
 *
 * @param inputTokens - 输入 Token
 * @param outputTokens - 输出 Token
 * @returns 冻结用量
 */
export function usageOf(inputTokens: number, outputTokens: number): TokenUsage {
  return createTokenUsage(inputTokens, outputTokens);
}

/** 事件收集结果。 */
export interface CollectedRun {
  /** 按顺序收集的事件。 */
  readonly events: readonly AgentEvent[];
  /** 生成器返回的最终结果。 */
  readonly result: AgentResult;
}

/**
 * 消费完整事件流。
 *
 * @param stream - Agent 事件流
 * @returns 事件数组与最终结果
 */
export async function collectEvents(
  stream: AsyncGenerator<AgentEvent, AgentResult, void>,
): Promise<CollectedRun> {
  const events: AgentEvent[] = [];
  let next = await stream.next();
  while (next.done !== true) {
    events.push(next.value);
    next = await stream.next();
  }
  return { events, result: next.value };
}

/**
 * 创建可手动完成的 Promise。
 *
 * @returns promise、resolve 与 reject
 */
export function deferred<TValue>(): {
  readonly promise: Promise<TValue>;
  readonly resolve: (value: TValue) => void;
  readonly reject: (reason?: unknown) => void;
} {
  let resolve!: (value: TValue) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<TValue>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

/**
 * 构造固定流事件的提供方。
 *
 * @param events - 流事件
 * @param fallback - generate 的响应，缺省抛出错误
 * @returns 提供方与调用计数
 */
export function createStreamingProvider(
  events: readonly ModelStreamEvent[],
  fallback?: ModelResponse,
): { readonly provider: ModelProvider; readonly counts: { stream: number; generate: number } } {
  const counts = { stream: 0, generate: 0 };
  return {
    counts,
    provider: {
      id: 'streaming',
      async generate(): Promise<ModelResponse> {
        counts.generate += 1;
        if (fallback === undefined) throw new Error('不应调用 generate');
        return fallback;
      },
      async *stream(): AsyncGenerator<ModelStreamEvent, void, void> {
        counts.stream += 1;
        for (const event of events) yield event;
      },
    },
  };
}

/**
 * 提取事件类型数组。
 *
 * @param events - Agent 事件
 * @returns 事件类型数组
 */
export function eventTypes(events: readonly AgentEvent[]): string[] {
  return events.map((event) => event.type);
}

/**
 * 构造包含指定内容的 assistant 消息响应。
 *
 * @param content - 内容块
 * @param stopReason - 停止原因
 * @returns 模型响应
 */
export function contentResponse(
  content: readonly AgentContentPart[],
  stopReason: ModelStopReason = ModelStopReason.End,
): ModelResponse {
  return { message: createAssistantMessage(content), stopReason };
}
