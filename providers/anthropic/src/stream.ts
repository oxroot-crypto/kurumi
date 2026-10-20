/**
 * @file Anthropic Messages SSE 流解析与增量转发。
 * @description 只处理 `data:` 行；支持 text/thinking/tool_use 内容块与最终 usage。
 * @module anthropic/stream
 */

import { isValidToolName, type ModelResponse, type ModelStreamEvent } from '@kurumi/agent';
import { isPlainObject } from '@kurumi/core';

import type { AnthropicStreamEvent, AnthropicUsage } from './dto.js';
import { AnthropicProviderError, redactAnthropicSecret } from './errors.js';
import type { ResolvedAnthropicProviderOptions } from './options.js';
import {
  buildAssistantMessage,
  mapStopReason,
  parseToolArguments,
  toUsage,
  type MappedContentBlock,
} from './response.js';

/** 单条 SSE 事件允许的最大缓冲字符数，防止无换行超长事件耗尽内存。 */
const MAX_SSE_BUFFER_LENGTH = 1_048_576;

/** 流式内容块累积器。 */
interface BlockAccumulator {
  readonly type: 'text' | 'thinking' | 'redacted_thinking' | 'tool_use';
  text: string;
  callId: string;
  name: string;
  argumentsText: string;
  signature: string;
  redactedData: string;
}

/** 流式 usage 累积器。 */
interface UsageAccumulator {
  inputTokens: number | undefined;
  outputTokens: number | undefined;
  cacheReadTokens: number | undefined;
  cacheWriteTokens: number | undefined;
}

/** 流解析状态。 */
interface StreamState {
  stopReason: string | undefined;
  readonly usage: UsageAccumulator;
  isDone: boolean;
}

/**
 * 消费 Anthropic SSE 响应并转发增量。
 *
 * @param response - 已确认成功的 HTTP 响应
 * @param options - 解析后的适配器选项
 * @returns 流事件；最后恰好一个 `response` 事件
 * @throws {AnthropicProviderError} body 缺失、SSE JSON 非法、事件字段非法或消息组装失败
 */
export async function* consumeStream(
  response: Response,
  options: ResolvedAnthropicProviderOptions,
): AsyncGenerator<ModelStreamEvent, void, void> {
  const reader = response.body?.getReader();
  if (reader === undefined || reader === null) {
    throw new AnthropicProviderError(options.id, '流式响应没有 body');
  }

  const decoder = new TextDecoder();
  const blocks = new Map<number, BlockAccumulator>();
  const state: StreamState = {
    stopReason: undefined,
    usage: {
      inputTokens: undefined,
      outputTokens: undefined,
      cacheReadTokens: undefined,
      cacheWriteTokens: undefined,
    },
    isDone: false,
  };
  let buffer = '';

  try {
    streamLoop: while (true) {
      let readResult: { readonly done: boolean; readonly value?: Uint8Array };
      try {
        readResult = await reader.read();
      } catch (cause) {
        throw new AnthropicProviderError(options.id, '读取流失败', { cause });
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
          throw new AnthropicProviderError(options.id, '流事件超过最大长度');
        }
        if (line.startsWith('data:')) {
          const payload = line.slice(5).trim();
          if (payload !== '') {
            const event = parseEvent(options, payload);
            yield* handleEvent(options, event, blocks, state);
            if (state.isDone) break streamLoop;
          }
        }
        newlineIndex = buffer.indexOf('\n');
      }

      if (buffer.length > MAX_SSE_BUFFER_LENGTH) {
        throw new AnthropicProviderError(options.id, '流事件超过最大长度');
      }
      if (done === true) break;
    }
  } finally {
    try {
      // 提前遇到 message_stop 或异常时取消底层 body，避免连接继续占用资源。
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

  yield {
    type: 'response',
    response: buildResponse(options, blocks, state),
  };
}

/** 根据累积器组装最终响应。 */
function buildResponse(
  options: ResolvedAnthropicProviderOptions,
  blocks: ReadonlyMap<number, BlockAccumulator>,
  state: StreamState,
): ModelResponse {
  const mapped: MappedContentBlock[] = [...blocks.entries()]
    .sort(([left], [right]) => left - right)
    .map(([, block]) => toMappedBlock(options, block));
  const usage = toAccumulatedUsage(options, state.usage);
  return {
    message: buildAssistantMessage(options, mapped),
    stopReason: mapStopReason(
      state.stopReason,
      mapped.some((block) => block.type === 'tool_call'),
    ),
    ...(usage === undefined ? {} : { usage }),
  };
}

/** 将累积块转换为 Agent 内容块。 */
function toMappedBlock(
  options: ResolvedAnthropicProviderOptions,
  block: BlockAccumulator,
): MappedContentBlock {
  switch (block.type) {
    case 'text':
      return { type: 'text', text: block.text };
    case 'thinking':
      return {
        type: 'reasoning',
        text: block.text,
        ...(block.signature === '' ? {} : { signature: block.signature }),
      };
    case 'redacted_thinking':
      return { type: 'reasoning', text: '', redactedData: block.redactedData };
    case 'tool_use':
      return {
        type: 'tool_call',
        callId: block.callId,
        name: block.name,
        input: parseToolArguments(options, block.argumentsText, block.name),
      };
  }
}

/** 将累积 usage 转换为 TokenUsage；没有任何计数时返回 undefined。 */
function toAccumulatedUsage(
  options: ResolvedAnthropicProviderOptions,
  usage: UsageAccumulator,
): ReturnType<typeof toUsage> | undefined {
  const raw: AnthropicUsage = {
    ...(usage.inputTokens === undefined ? {} : { input_tokens: usage.inputTokens }),
    ...(usage.outputTokens === undefined ? {} : { output_tokens: usage.outputTokens }),
    ...(usage.cacheReadTokens === undefined
      ? {}
      : { cache_read_input_tokens: usage.cacheReadTokens }),
    ...(usage.cacheWriteTokens === undefined
      ? {}
      : { cache_creation_input_tokens: usage.cacheWriteTokens }),
  };
  if (Object.keys(raw).length === 0) return undefined;
  return toUsage(options, raw);
}

/** 处理单个 SSE 事件。 */
function* handleEvent(
  options: ResolvedAnthropicProviderOptions,
  event: AnthropicStreamEvent,
  blocks: Map<number, BlockAccumulator>,
  state: StreamState,
): Generator<ModelStreamEvent, void, void> {
  switch (event.type) {
    case 'message_start':
      handleMessageStart(options, event, state);
      return;
    case 'content_block_start':
      yield* handleContentBlockStart(options, event, blocks);
      return;
    case 'content_block_delta':
      yield* handleContentBlockDelta(options, event, blocks);
      return;
    case 'content_block_stop':
      handleContentBlockStop(options, event, blocks);
      return;
    case 'message_delta':
      handleMessageDelta(options, event, state);
      return;
    case 'message_stop':
      state.isDone = true;
      return;
    case 'ping':
      return;
    case 'error':
      throw new AnthropicProviderError(options.id, streamErrorMessage(options, event['error']));
    default:
      throw new AnthropicProviderError(options.id, `未知流事件类型 "${String(event.type)}"`);
  }
}

/** 处理 message_start。 */
function handleMessageStart(
  options: ResolvedAnthropicProviderOptions,
  event: AnthropicStreamEvent,
  state: StreamState,
): void {
  const message = event['message'];
  if (!isPlainObject(message)) {
    throw new AnthropicProviderError(options.id, 'message_start.message 必须是对象');
  }
  const usage = message['usage'];
  if (usage !== undefined && usage !== null) {
    mergeUsage(options, usage, state.usage, 'message_start.message.usage');
  }
}

/** 处理 content_block_start。 */
function* handleContentBlockStart(
  options: ResolvedAnthropicProviderOptions,
  event: AnthropicStreamEvent,
  blocks: Map<number, BlockAccumulator>,
): Generator<ModelStreamEvent, void, void> {
  const index = readIndex(options, event['index']);
  if (blocks.has(index)) {
    throw new AnthropicProviderError(options.id, `content_block_start.index ${String(index)} 重复`);
  }
  const value = event['content_block'];
  if (!isPlainObject(value)) {
    throw new AnthropicProviderError(options.id, 'content_block_start.content_block 必须是对象');
  }
  const type = value['type'];
  if (typeof type !== 'string') {
    throw new AnthropicProviderError(options.id, 'content_block.type 必须是字符串');
  }

  switch (type) {
    case 'text': {
      const text = readOptionalString(options, value['text'], 'content_block.text') ?? '';
      blocks.set(index, {
        type: 'text',
        text,
        callId: '',
        name: '',
        argumentsText: '',
        signature: '',
        redactedData: '',
      });
      if (text !== '') yield { type: 'text_delta', text };
      return;
    }
    case 'thinking': {
      const thinking =
        readOptionalString(options, value['thinking'], 'content_block.thinking') ?? '';
      const signature =
        readOptionalString(options, value['signature'], 'content_block.signature') ?? '';
      blocks.set(index, {
        type: 'thinking',
        text: thinking,
        callId: '',
        name: '',
        argumentsText: '',
        signature,
        redactedData: '',
      });
      if (thinking !== '') yield { type: 'reasoning_delta', text: thinking };
      return;
    }
    case 'redacted_thinking': {
      const data = readRequiredString(options, value['data'], 'content_block.data');
      blocks.set(index, {
        type: 'redacted_thinking',
        text: '',
        callId: '',
        name: '',
        argumentsText: '',
        signature: '',
        redactedData: data,
      });
      return;
    }
    case 'tool_use': {
      const callId = readRequiredString(options, value['id'], 'content_block.id');
      const name = readRequiredString(options, value['name'], 'content_block.name');
      if (!isValidToolName(name)) {
        throw new AnthropicProviderError(options.id, 'content_block.name 必须匹配工具名称规则');
      }
      const input = value['input'];
      if (!isPlainObject(input) || Object.keys(input).length > 0) {
        throw new AnthropicProviderError(options.id, '流式 tool_use 的起始 input 必须是空对象');
      }
      blocks.set(index, {
        type: 'tool_use',
        text: '',
        callId,
        name,
        argumentsText: '',
        signature: '',
        redactedData: '',
      });
      yield { type: 'tool_call_delta', index, callId, name, argumentsDelta: '' };
      return;
    }
    default:
      throw new AnthropicProviderError(options.id, `content_block.type "${type}" 不受支持`);
  }
}

/** 处理 content_block_delta。 */
function* handleContentBlockDelta(
  options: ResolvedAnthropicProviderOptions,
  event: AnthropicStreamEvent,
  blocks: Map<number, BlockAccumulator>,
): Generator<ModelStreamEvent, void, void> {
  const index = readIndex(options, event['index']);
  const block = blocks.get(index);
  if (block === undefined) {
    throw new AnthropicProviderError(
      options.id,
      `content_block_delta.index ${String(index)} 没有对应内容块`,
    );
  }
  const delta = event['delta'];
  if (!isPlainObject(delta)) {
    throw new AnthropicProviderError(options.id, 'content_block_delta.delta 必须是对象');
  }
  const type = delta['type'];
  if (typeof type !== 'string') {
    throw new AnthropicProviderError(options.id, 'delta.type 必须是字符串');
  }

  switch (type) {
    case 'text_delta': {
      assertBlockType(options, block, 'text', type);
      const text = readString(options, delta['text'], 'delta.text');
      block.text += text;
      yield { type: 'text_delta', text };
      return;
    }
    case 'thinking_delta': {
      assertBlockType(options, block, 'thinking', type);
      const thinking = readString(options, delta['thinking'], 'delta.thinking');
      block.text += thinking;
      yield { type: 'reasoning_delta', text: thinking };
      return;
    }
    case 'input_json_delta': {
      assertBlockType(options, block, 'tool_use', type);
      const partialJson = readString(options, delta['partial_json'], 'delta.partial_json');
      block.argumentsText += partialJson;
      yield { type: 'tool_call_delta', index, argumentsDelta: partialJson };
      return;
    }
    case 'signature_delta': {
      assertBlockType(options, block, 'thinking', type);
      block.signature += readString(options, delta['signature'], 'delta.signature');
      return;
    }
    default:
      throw new AnthropicProviderError(options.id, `delta.type "${type}" 不受支持`);
  }
}

/** 处理 content_block_stop。 */
function handleContentBlockStop(
  options: ResolvedAnthropicProviderOptions,
  event: AnthropicStreamEvent,
  blocks: Map<number, BlockAccumulator>,
): void {
  const index = readIndex(options, event['index']);
  if (!blocks.has(index)) {
    throw new AnthropicProviderError(
      options.id,
      `content_block_stop.index ${String(index)} 没有对应内容块`,
    );
  }
}

/** 处理 message_delta。 */
function handleMessageDelta(
  options: ResolvedAnthropicProviderOptions,
  event: AnthropicStreamEvent,
  state: StreamState,
): void {
  const delta = event['delta'];
  if (!isPlainObject(delta)) {
    throw new AnthropicProviderError(options.id, 'message_delta.delta 必须是对象');
  }
  const stopReason = delta['stop_reason'];
  if (stopReason !== undefined && stopReason !== null) {
    if (typeof stopReason !== 'string') {
      throw new AnthropicProviderError(
        options.id,
        'message_delta.delta.stop_reason 必须是字符串或 null',
      );
    }
    state.stopReason = stopReason;
  }
  const usage = event['usage'];
  if (usage !== undefined && usage !== null) {
    mergeUsage(options, usage, state.usage, 'message_delta.usage');
  }
}

/** 合并 usage 字段；缺省或 null 不覆盖已有值。 */
function mergeUsage(
  options: ResolvedAnthropicProviderOptions,
  value: unknown,
  target: UsageAccumulator,
  field: string,
): void {
  if (!isPlainObject(value)) {
    throw new AnthropicProviderError(options.id, `${field} 必须是对象`);
  }
  const inputTokens = readOptionalCount(options, value['input_tokens'], `${field}.input_tokens`);
  const outputTokens = readOptionalCount(options, value['output_tokens'], `${field}.output_tokens`);
  const cacheReadTokens = readOptionalCount(
    options,
    value['cache_read_input_tokens'],
    `${field}.cache_read_input_tokens`,
  );
  const cacheWriteTokens = readOptionalCount(
    options,
    value['cache_creation_input_tokens'],
    `${field}.cache_creation_input_tokens`,
  );
  if (inputTokens !== undefined) target.inputTokens = inputTokens;
  if (outputTokens !== undefined) target.outputTokens = outputTokens;
  if (cacheReadTokens !== undefined) target.cacheReadTokens = cacheReadTokens;
  if (cacheWriteTokens !== undefined) target.cacheWriteTokens = cacheWriteTokens;
}

/** 断言 delta 类型与内容块类型匹配。 */
function assertBlockType(
  options: ResolvedAnthropicProviderOptions,
  block: BlockAccumulator,
  expected: BlockAccumulator['type'],
  deltaType: string,
): void {
  if (block.type !== expected) {
    throw new AnthropicProviderError(
      options.id,
      `delta.type "${deltaType}" 与内容块类型 "${block.type}" 不匹配`,
    );
  }
}

/** 解析 SSE 数据行。 */
function parseEvent(
  options: ResolvedAnthropicProviderOptions,
  payload: string,
): AnthropicStreamEvent {
  try {
    const value: unknown = JSON.parse(payload);
    if (!isPlainObject(value)) throw new Error('不是对象');
    const type = value['type'];
    if (typeof type !== 'string') throw new Error('缺少 type');
    return value as AnthropicStreamEvent;
  } catch (error) {
    throw new AnthropicProviderError(options.id, '流事件不是合法 JSON', { cause: error });
  }
}

/** 读取必需的非负安全整数下标。 */
function readIndex(options: ResolvedAnthropicProviderOptions, value: unknown): number {
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0) {
    throw new AnthropicProviderError(options.id, '内容块 index 必须是非负安全整数');
  }
  return value;
}

/** 读取必需的非空字符串。 */
function readRequiredString(
  options: ResolvedAnthropicProviderOptions,
  value: unknown,
  field: string,
): string {
  if (typeof value !== 'string' || value.trim() === '') {
    throw new AnthropicProviderError(options.id, `${field} 必须是非空字符串`);
  }
  return value;
}

/** 读取必需字符串；允许空字符串。 */
function readString(
  options: ResolvedAnthropicProviderOptions,
  value: unknown,
  field: string,
): string {
  if (typeof value !== 'string') {
    throw new AnthropicProviderError(options.id, `${field} 必须是字符串`);
  }
  return value;
}

/** 读取可选字符串；缺省或 null 返回 undefined。 */
function readOptionalString(
  options: ResolvedAnthropicProviderOptions,
  value: unknown,
  field: string,
): string | undefined {
  if (value === undefined || value === null) return undefined;
  if (typeof value !== 'string') {
    throw new AnthropicProviderError(options.id, `${field} 必须是字符串`);
  }
  return value;
}

/** 读取可选非负安全整数；缺省或 null 返回 undefined。 */
function readOptionalCount(
  options: ResolvedAnthropicProviderOptions,
  value: unknown,
  field: string,
): number | undefined {
  if (value === undefined || value === null) return undefined;
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0) {
    throw new AnthropicProviderError(options.id, `${field} 必须是非负安全整数`);
  }
  return value;
}

/** 从 SSE error 字段提取可诊断但长度受限的消息。 */
function streamErrorMessage(options: ResolvedAnthropicProviderOptions, value: unknown): string {
  if (isPlainObject(value)) {
    const message = value['message'];
    if (typeof message === 'string' && message.trim() !== '') {
      return `流返回错误: ${redactAnthropicSecret(message, options.apiKey).slice(0, 1_000)}`;
    }
  }
  return '流返回错误';
}
