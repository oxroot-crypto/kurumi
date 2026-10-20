/**
 * @file Anthropic 响应到 Agent 消息的映射与共享校验。
 * @description 非流式响应和流式最终组装共用同一套内容块、工具与 usage 校验。
 * @module anthropic/response
 */

import {
  ModelStopReason,
  createAssistantMessage,
  createTokenUsage,
  isValidToolName,
  type AgentContentPart,
  type AgentMessage,
  type ModelResponse,
  type TokenUsage,
} from '@kurumi/agent';
import { cloneJsonValue, isPlainObject, type JsonValue } from '@kurumi/core';

import type { AnthropicResponseContentBlock, AnthropicResponseToolUseBlock } from './dto.js';
import { AnthropicProviderError } from './errors.js';
import type { ResolvedAnthropicProviderOptions } from './options.js';

/** 响应内容块映射后的中间结构。 */
export type MappedContentBlock =
  | { readonly type: 'text'; readonly text: string }
  | {
      readonly type: 'reasoning';
      readonly text: string;
      readonly signature?: string;
      readonly redactedData?: string;
    }
  | {
      readonly type: 'tool_call';
      readonly callId: string;
      readonly name: string;
      readonly input: JsonValue;
    };

/**
 * 映射非流式响应。
 *
 * @param options - 解析后的适配器选项
 * @param value - 未经信任的响应 JSON
 * @returns 已校验的模型响应
 * @throws {AnthropicProviderError} 响应结构、内容块、工具输入或 usage 非法
 */
export function toModelResponse(
  options: ResolvedAnthropicProviderOptions,
  value: unknown,
): ModelResponse {
  if (!isPlainObject(value)) {
    throw new AnthropicProviderError(options.id, '响应必须是普通对象');
  }
  const role = value['role'];
  if (role !== undefined && role !== 'assistant') {
    throw new AnthropicProviderError(options.id, '响应 role 必须是 assistant');
  }
  const type = value['type'];
  if (type !== undefined && type !== 'message') {
    throw new AnthropicProviderError(options.id, '响应 type 必须是 message');
  }
  const content = value['content'];
  if (!Array.isArray(content)) {
    throw new AnthropicProviderError(options.id, '响应 content 必须是数组');
  }
  const blocks = content.map((block, index) => {
    return toMappedBlock(options, block, `content[${String(index)}]`);
  });
  const rawUsage = value['usage'];
  const usage =
    rawUsage === undefined || rawUsage === null ? undefined : toUsage(options, rawUsage);
  const stopReason = readStopReason(options, value['stop_reason']);

  return {
    message: buildAssistantMessage(options, blocks),
    stopReason: mapStopReason(
      stopReason,
      blocks.some((block) => block.type === 'tool_call'),
    ),
    ...(usage === undefined ? {} : { usage }),
  };
}

/**
 * 构造 assistant 消息。
 *
 * @param options - 解析后的适配器选项
 * @param blocks - 按协议顺序排列的内容块
 * @returns 冻结的 assistant 消息
 * @throws {AnthropicProviderError} 消息结构非法
 */
export function buildAssistantMessage(
  options: ResolvedAnthropicProviderOptions,
  blocks: readonly MappedContentBlock[],
): AgentMessage {
  const parts: AgentContentPart[] = blocks.map((block): AgentContentPart => {
    switch (block.type) {
      case 'text':
        return { type: 'text', text: block.text };
      case 'reasoning':
        return {
          type: 'reasoning',
          text: block.text,
          ...(block.signature === undefined ? {} : { signature: block.signature }),
          ...(block.redactedData === undefined ? {} : { redactedData: block.redactedData }),
        };
      case 'tool_call':
        return {
          type: 'tool_call',
          callId: block.callId,
          name: block.name,
          input: block.input,
        };
    }
  });
  const hasTextOrToolCall = blocks.some(
    (block) => block.type === 'text' || block.type === 'tool_call',
  );
  if (!hasTextOrToolCall) parts.push({ type: 'text', text: '' });

  try {
    return createAssistantMessage(parts);
  } catch (error) {
    throw new AnthropicProviderError(options.id, '返回了非法消息', { cause: error });
  }
}

/**
 * 解析流式工具参数 JSON。
 *
 * @param options - 解析后的适配器选项
 * @param raw - 模型返回的原始参数文本
 * @param toolName - 工具名称，用于诊断
 * @returns JSON 安全值；空字符串视为 `{}`
 * @throws {AnthropicProviderError} 参数不是合法 JSON
 */
export function parseToolArguments(
  options: ResolvedAnthropicProviderOptions,
  raw: string,
  toolName: string,
): JsonValue {
  const text = raw.trim();
  if (text === '') return {};
  try {
    return JSON.parse(text) as JsonValue;
  } catch (error) {
    throw new AnthropicProviderError(options.id, `工具 "${toolName}" 的参数不是合法 JSON`, {
      cause: error,
    });
  }
}

/**
 * 映射 usage。
 *
 * @param options - 解析后的适配器选项
 * @param usage - 原始 usage
 * @returns 冻结的 Token 用量
 * @throws {AnthropicProviderError} 计数非法或总和溢出
 */
export function toUsage(options: ResolvedAnthropicProviderOptions, usage: unknown): TokenUsage {
  if (!isPlainObject(usage)) {
    throw new AnthropicProviderError(options.id, 'usage 必须是普通对象');
  }
  const inputTokens = readCount(options, usage['input_tokens'], 'usage.input_tokens') ?? 0;
  const outputTokens = readCount(options, usage['output_tokens'], 'usage.output_tokens') ?? 0;
  const cacheWriteTokens = readCount(
    options,
    usage['cache_creation_input_tokens'],
    'usage.cache_creation_input_tokens',
  );
  const cacheReadTokens = readCount(
    options,
    usage['cache_read_input_tokens'],
    'usage.cache_read_input_tokens',
  );
  const totalInputTokens = inputTokens + (cacheWriteTokens ?? 0) + (cacheReadTokens ?? 0);
  if (!Number.isSafeInteger(totalInputTokens)) {
    throw new AnthropicProviderError(options.id, 'usage 输入 Token 总和溢出安全整数');
  }

  try {
    return createTokenUsage(totalInputTokens, outputTokens, {
      ...(cacheReadTokens === undefined ? {} : { cacheReadTokens }),
      ...(cacheWriteTokens === undefined ? {} : { cacheWriteTokens }),
    });
  } catch (error) {
    throw new AnthropicProviderError(options.id, '返回了非法 usage', { cause: error });
  }
}

/**
 * 映射停止原因。
 *
 * @param reason - 原始 `stop_reason`
 * @param hasToolCalls - 是否包含工具调用
 * @returns Agent 模型停止原因
 */
export function mapStopReason(reason: string | undefined, hasToolCalls: boolean): ModelStopReason {
  switch (reason) {
    case 'max_tokens':
      return ModelStopReason.MaxTokens;
    case 'refusal':
      return ModelStopReason.ContentFilter;
    case 'tool_use':
      return ModelStopReason.ToolCalls;
    case 'end_turn':
    case 'stop_sequence':
      return hasToolCalls ? ModelStopReason.ToolCalls : ModelStopReason.End;
    default:
      return hasToolCalls ? ModelStopReason.ToolCalls : ModelStopReason.Other;
  }
}

/** 映射单个响应内容块。 */
function toMappedBlock(
  options: ResolvedAnthropicProviderOptions,
  value: unknown,
  path: string,
): MappedContentBlock {
  if (!isPlainObject(value)) {
    throw new AnthropicProviderError(options.id, `${path} 必须是对象`);
  }
  const block = value as AnthropicResponseContentBlock;
  switch (block.type) {
    case 'text': {
      const text = block['text'];
      if (typeof text !== 'string') {
        throw new AnthropicProviderError(options.id, `${path}.text 必须是字符串`);
      }
      return { type: 'text', text };
    }
    case 'thinking': {
      const thinking = block['thinking'];
      if (typeof thinking !== 'string') {
        throw new AnthropicProviderError(options.id, `${path}.thinking 必须是字符串`);
      }
      const signature = readOptionalNonEmptyString(
        options,
        block['signature'],
        `${path}.signature`,
      );
      return {
        type: 'reasoning',
        text: thinking,
        ...(signature === undefined ? {} : { signature }),
      };
    }
    case 'redacted_thinking': {
      const data = readRequiredNonEmptyString(options, block['data'], `${path}.data`);
      return { type: 'reasoning', text: '', redactedData: data };
    }
    case 'tool_use':
      return toToolCallBlock(options, block as AnthropicResponseToolUseBlock, path);
    default:
      throw new AnthropicProviderError(options.id, `${path}.type "${String(block.type)}" 不受支持`);
  }
}

/** 映射 tool_use 内容块。 */
function toToolCallBlock(
  options: ResolvedAnthropicProviderOptions,
  block: AnthropicResponseToolUseBlock,
  path: string,
): MappedContentBlock {
  const callId = block['id'];
  if (typeof callId !== 'string' || callId.trim() === '') {
    throw new AnthropicProviderError(options.id, `${path}.id 必须是非空字符串`);
  }
  const name = block['name'];
  if (!isValidToolName(name)) {
    throw new AnthropicProviderError(options.id, `${path}.name 必须匹配工具名称规则`);
  }
  const input = block['input'];
  if (input === undefined) {
    throw new AnthropicProviderError(options.id, `${path}.input 必须是 JSON 安全值`);
  }
  let jsonInput: JsonValue;
  try {
    jsonInput = cloneJsonValue(input, `${path}.input`);
  } catch (error) {
    throw new AnthropicProviderError(options.id, `${path}.input 必须是 JSON 安全值`, {
      cause: error,
    });
  }
  return { type: 'tool_call', callId, name, input: jsonInput };
}

/** 读取并校验 stop_reason。 */
function readStopReason(
  options: ResolvedAnthropicProviderOptions,
  value: unknown,
): string | undefined {
  if (value === undefined || value === null) return undefined;
  if (typeof value !== 'string') {
    throw new AnthropicProviderError(options.id, 'stop_reason 必须是字符串或 null');
  }
  return value;
}

/** 读取可选非负安全整数；null/undefined 视为缺省。 */
function readCount(
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

/** 读取可选非空字符串；null/undefined 视为缺省。 */
function readOptionalNonEmptyString(
  options: ResolvedAnthropicProviderOptions,
  value: unknown,
  field: string,
): string | undefined {
  if (value === undefined || value === null) return undefined;
  return readRequiredNonEmptyString(options, value, field);
}

/** 读取必需的非空字符串。 */
function readRequiredNonEmptyString(
  options: ResolvedAnthropicProviderOptions,
  value: unknown,
  field: string,
): string {
  if (typeof value !== 'string' || value.trim() === '') {
    throw new AnthropicProviderError(options.id, `${field} 必须是非空字符串`);
  }
  return value;
}
