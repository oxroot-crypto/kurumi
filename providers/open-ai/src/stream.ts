/**
 * @file SSE 流解析与 OpenAI 流式增量转发。
 * @description 只处理 `data:` 行；支持文本、可配置推理字段、工具参数增量与最终 usage。
 * @module open-ai/stream
 */

import type { ModelStreamEvent, TokenUsage } from '@kurumi/agent';
import { isPlainObject } from '@kurumi/core';

import type { OpenAIChunk, OpenAIStreamDelta, OpenAIStreamToolCallDelta } from './dto.js';
import { OpenAIProviderError } from './errors.js';
import type { ResolvedOpenAIProviderOptions } from './options.js';
import {
  buildAssistantMessage,
  mapStopReason,
  toUsage,
  type AccumulatedToolCall,
} from './response.js';

/** 单条 SSE 事件允许的最大缓冲字符数，防止无换行超长事件耗尽内存。 */
const MAX_SSE_BUFFER_LENGTH = 1_048_576;

/** 流式工具调用累积器。 */
interface ToolCallAccumulator {
  id: string;
  name: string;
  argumentsText: string;
}

/**
 * 消费 SSE 响应并转发增量。
 *
 * @param response - 已确认成功的 HTTP 响应
 * @param options - 解析后的适配器选项
 * @returns 流事件；最后恰好一个 `response` 事件
 * @throws {OpenAIProviderError} body 缺失、SSE JSON 非法、delta 字段非法或消息组装失败
 */
export async function* consumeStream(
  response: Response,
  options: ResolvedOpenAIProviderOptions,
): AsyncGenerator<ModelStreamEvent, void, void> {
  const reader = response.body?.getReader();
  if (reader === undefined || reader === null) {
    throw new OpenAIProviderError(options.id, '流式响应没有 body');
  }

  const decoder = new TextDecoder();
  let buffer = '';
  let text = '';
  let reasoning = '';
  let finishReason: string | undefined;
  let usage: TokenUsage | undefined;
  const toolCalls = new Map<number, ToolCallAccumulator>();

  try {
    streamLoop: while (true) {
      let readResult: { readonly done: boolean; readonly value?: Uint8Array };
      try {
        readResult = await reader.read();
      } catch (cause) {
        throw new OpenAIProviderError(options.id, '读取流失败', { cause });
      }
      const { done, value } = readResult;
      if (done === true) {
        buffer += `${decoder.decode()}\n`;
      } else {
        buffer += decoder.decode(value, { stream: true });
      }

      let newlineIndex = buffer.indexOf('\n');
      while (newlineIndex >= 0) {
        const line = buffer.slice(0, newlineIndex).replace(/\r$/, '');
        buffer = buffer.slice(newlineIndex + 1);
        if (line.length > MAX_SSE_BUFFER_LENGTH) {
          throw new OpenAIProviderError(options.id, '流事件超过最大长度');
        }
        if (line.startsWith('data:')) {
          const payload = line.slice(5).trim();
          if (payload === '[DONE]') break streamLoop;
          if (payload !== '') {
            const chunk = parseChunk(options, payload);
            if (chunk['error'] !== undefined) {
              throw new OpenAIProviderError(options.id, streamErrorMessage(chunk['error']));
            }
            const hasUsageField = chunk['usage'] !== undefined;
            if (hasUsageField && chunk['usage'] !== null) {
              usage = toUsage(options, chunk['usage']);
            }
            const rawChoices = chunk['choices'];
            if (rawChoices !== undefined && !Array.isArray(rawChoices)) {
              throw new OpenAIProviderError(options.id, 'choices 必须是数组');
            }
            if (rawChoices === undefined && !hasUsageField) {
              throw new OpenAIProviderError(options.id, '流事件缺少 choices 或 usage');
            }
            const choice = rawChoices?.[0];
            if (choice !== undefined && !isPlainObject(choice)) {
              throw new OpenAIProviderError(options.id, 'choices[0] 必须是对象');
            }
            if (choice?.['finish_reason'] !== undefined && choice['finish_reason'] !== null) {
              finishReason = readFinishReason(options, choice['finish_reason']);
            }
            const delta = choice?.['delta'];
            if (delta !== undefined && delta !== null && !isPlainObject(delta)) {
              throw new OpenAIProviderError(options.id, 'choices[0].delta 必须是对象');
            }
            yield* applyDelta(options, delta, toolCalls, {
              appendText(value2): void {
                text += value2;
              },
              appendReasoning(value2): void {
                reasoning += value2;
              },
            });
          }
        }
        newlineIndex = buffer.indexOf('\n');
      }

      if (buffer.length > MAX_SSE_BUFFER_LENGTH) {
        throw new OpenAIProviderError(options.id, '流事件超过最大长度');
      }
      if (done === true) break;
    }
  } finally {
    try {
      // 提前遇到 [DONE] 或异常时取消底层 body，避免连接继续占用资源。
      await reader.cancel();
    } catch {
      // 已完成或底层已关闭时 cancel 可能失败，不覆盖原始错误。
    }
    try {
      reader.releaseLock();
    } catch {
      // release 失败不应覆盖原始流错误。
    }
  }

  const calls: AccumulatedToolCall[] = [...toolCalls.entries()]
    .sort(([left], [right]) => left - right)
    .map(([, call]) => ({ id: call.id, name: call.name, argumentsText: call.argumentsText }));

  yield {
    type: 'response',
    response: {
      message: buildAssistantMessage(options, text, reasoning, calls),
      stopReason: mapStopReason(finishReason, calls.length > 0),
      ...(usage === undefined ? {} : { usage }),
    },
  };
}

/** 应用 delta 并转发事件。 */
function* applyDelta(
  options: ResolvedOpenAIProviderOptions,
  delta: OpenAIStreamDelta | undefined,
  toolCalls: Map<number, ToolCallAccumulator>,
  sink: {
    appendText(value: string): void;
    appendReasoning(value: string): void;
  },
): Generator<ModelStreamEvent, void, void> {
  if (delta === undefined || delta === null) return;
  if (!isPlainObject(delta)) {
    throw new OpenAIProviderError(options.id, 'choices[0].delta 必须是对象');
  }

  const reasoningDelta = readReasoningDelta(options, delta);
  if (reasoningDelta !== undefined) {
    sink.appendReasoning(reasoningDelta);
    yield { type: 'reasoning_delta', text: reasoningDelta };
  }

  if (delta['content'] !== undefined && delta['content'] !== null) {
    const content = delta['content'];
    if (typeof content !== 'string') {
      throw new OpenAIProviderError(options.id, 'delta.content 必须是字符串或 null');
    }
    sink.appendText(content);
    yield { type: 'text_delta', text: content };
  }

  const calls = delta['tool_calls'] ?? [];
  if (!Array.isArray(calls)) {
    throw new OpenAIProviderError(options.id, 'delta.tool_calls 必须是数组');
  }
  for (let position = 0; position < calls.length; position += 1) {
    const call = calls[position];
    if (call === undefined) continue;
    if (!isPlainObject(call)) {
      throw new OpenAIProviderError(options.id, 'delta.tool_calls 元素必须是对象');
    }
    const streamCall = call as OpenAIStreamToolCallDelta;
    const index = readIndex(options, streamCall['index'], position);
    const accumulator = toolCalls.get(index) ?? { id: '', name: '', argumentsText: '' };
    const callId = streamCall['id'];
    if (callId !== undefined) {
      if (typeof callId !== 'string' || callId === '') {
        throw new OpenAIProviderError(options.id, 'delta.tool_calls[].id 必须是非空字符串');
      }
      accumulator.id = callId;
    }
    const functionName = streamCall['function']?.['name'];
    if (functionName !== undefined) {
      if (typeof functionName !== 'string' || functionName === '') {
        throw new OpenAIProviderError(
          options.id,
          'delta.tool_calls[].function.name 必须是非空字符串',
        );
      }
      accumulator.name = functionName;
    }
    const argumentsDelta = readArgumentsDelta(options, streamCall);
    accumulator.argumentsText += argumentsDelta;
    toolCalls.set(index, accumulator);

    yield {
      type: 'tool_call_delta',
      index,
      ...(callId === undefined ? {} : { callId }),
      ...(functionName === undefined ? {} : { name: functionName }),
      argumentsDelta,
    };
  }
}

/** 读取推理增量字段。 */
function readReasoningDelta(
  options: ResolvedOpenAIProviderOptions,
  delta: OpenAIStreamDelta,
): string | undefined {
  const value = delta[options.reasoningContentField];
  if (value === undefined || value === null) return undefined;
  if (typeof value !== 'string') {
    throw new OpenAIProviderError(
      options.id,
      `delta.${options.reasoningContentField} 必须是字符串`,
    );
  }
  return value;
}

/** 读取工具调用下标。 */
function readIndex(
  options: ResolvedOpenAIProviderOptions,
  value: number | undefined,
  fallback: number,
): number {
  if (value === undefined) return fallback;
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new OpenAIProviderError(options.id, 'delta.tool_calls[].index 必须是非负安全整数');
  }
  return value;
}

/** 读取工具参数增量。 */
function readArgumentsDelta(
  options: ResolvedOpenAIProviderOptions,
  call: OpenAIStreamToolCallDelta,
): string {
  const value = call['function']?.['arguments'];
  if (value === undefined) return '';
  if (typeof value !== 'string') {
    throw new OpenAIProviderError(options.id, 'delta.tool_calls[].function.arguments 必须是字符串');
  }
  return value;
}

/** 解析 SSE 数据行。 */
function parseChunk(options: ResolvedOpenAIProviderOptions, payload: string): OpenAIChunk {
  try {
    const value: unknown = JSON.parse(payload);
    if (!isPlainObject(value)) throw new Error('不是对象');
    return value as OpenAIChunk;
  } catch (error) {
    throw new OpenAIProviderError(options.id, '流事件不是合法 JSON', { cause: error });
  }
}

/** 校验 finish_reason。 */
function readFinishReason(options: ResolvedOpenAIProviderOptions, value: unknown): string {
  if (typeof value !== 'string') {
    throw new OpenAIProviderError(options.id, 'finish_reason 必须是字符串');
  }
  return value;
}

/** 从 SSE error 字段提取可诊断但长度受限的消息。 */
function streamErrorMessage(value: unknown): string {
  if (isPlainObject(value)) {
    const message = value['message'];
    if (typeof message === 'string' && message.trim() !== '') {
      return `流返回错误: ${message.slice(0, 1_000)}`;
    }
  }
  return '流返回错误';
}
