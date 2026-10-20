/**
 * @file OpenAI 响应到 Agent 消息的映射与共享校验。
 * @description 非流式响应和流式最终组装共用同一套消息、工具与 usage 校验。
 * @module open-ai/response
 */

import {
  ModelStopReason,
  createAssistantMessage,
  createTokenUsage,
  type AgentContentPart,
  type AgentMessage,
  type ModelResponse,
  type TokenUsage,
} from '@kurumi/agent';
import { isPlainObject, type JsonValue } from '@kurumi/core';

import type {
  OpenAIChoice,
  OpenAIChunk,
  OpenAIResponseMessage,
  OpenAIResponseToolCall,
  OpenAIUsage,
} from './dto.js';
import { OpenAIProviderError } from './errors.js';
import type { ResolvedOpenAIProviderOptions } from './options.js';

/** 流式工具调用累积结果。 */
export interface AccumulatedToolCall {
  /** 调用 ID。 */
  readonly id: string;
  /** 工具名称。 */
  readonly name: string;
  /** 拼接后的参数 JSON 文本。 */
  readonly argumentsText: string;
}

/**
 * 映射非流式响应。
 *
 * @param options - 解析后的适配器选项
 * @param value - 未经信任的响应 JSON
 * @returns 已校验的模型响应
 * @throws {OpenAIProviderError} 响应结构、消息、工具参数或 usage 非法
 */
export function toModelResponse(
  options: ResolvedOpenAIProviderOptions,
  value: unknown,
): ModelResponse {
  if (!isPlainObject(value)) {
    throw new OpenAIProviderError(options.id, '响应必须是普通对象');
  }
  const chunk = value as OpenAIChunk;
  const choice = chunk['choices']?.[0];
  if (choice === undefined || !isPlainObject(choice)) {
    throw new OpenAIProviderError(options.id, '响应缺少 choices[0]');
  }
  const responseChoice = choice as OpenAIChoice;
  const rawMessage = responseChoice['message'];
  if (rawMessage === undefined || !isPlainObject(rawMessage)) {
    throw new OpenAIProviderError(options.id, '响应缺少 choices[0].message');
  }
  const message = rawMessage as OpenAIResponseMessage;

  const text = readText(options, message);
  const reasoning = readReasoning(options, message);
  const calls = readToolCalls(options, message['tool_calls']);
  const rawUsage = chunk['usage'];
  const usage =
    rawUsage === undefined || rawUsage === null ? undefined : toUsage(options, rawUsage);
  const finishReason = readFinishReason(options, responseChoice['finish_reason']);

  return {
    message: buildAssistantMessage(options, text, reasoning, calls),
    stopReason: mapStopReason(finishReason, calls.length > 0),
    ...(usage === undefined ? {} : { usage }),
  };
}

/**
 * 构造 assistant 消息。
 *
 * @param options - 解析后的适配器选项
 * @param text - 文本内容
 * @param reasoning - 推理内容
 * @param calls - 工具调用累积结果
 * @returns 冻结的 assistant 消息
 * @throws {OpenAIProviderError} 工具参数非法或消息结构非法
 */
export function buildAssistantMessage(
  options: ResolvedOpenAIProviderOptions,
  text: string,
  reasoning: string,
  calls: readonly AccumulatedToolCall[],
): AgentMessage {
  const parts: AgentContentPart[] = [];
  if (reasoning !== '') parts.push({ type: 'reasoning', text: reasoning });
  if (text !== '') parts.push({ type: 'text', text });
  for (const call of calls) {
    parts.push({
      type: 'tool_call',
      callId: call.id,
      name: call.name,
      input: parseToolArguments(options, call.argumentsText, call.name),
    });
  }
  if (parts.length === 0) parts.push({ type: 'text', text: '' });

  try {
    return createAssistantMessage(parts);
  } catch (error) {
    throw new OpenAIProviderError(options.id, '返回了非法消息', { cause: error });
  }
}

/**
 * 解析工具参数 JSON。
 *
 * @param options - 解析后的适配器选项
 * @param raw - 模型返回的原始参数文本
 * @param toolName - 工具名称，用于诊断
 * @returns JSON 安全值；空字符串视为 `{}`
 * @throws {OpenAIProviderError} 参数不是合法 JSON
 */
export function parseToolArguments(
  options: ResolvedOpenAIProviderOptions,
  raw: string,
  toolName: string,
): JsonValue {
  const text = raw.trim();
  if (text === '') return {};
  try {
    return JSON.parse(text) as JsonValue;
  } catch (error) {
    throw new OpenAIProviderError(options.id, `工具 "${toolName}" 的参数不是合法 JSON`, {
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
 * @throws {OpenAIProviderError} 计数非法或总和溢出
 */
export function toUsage(options: ResolvedOpenAIProviderOptions, usage: OpenAIUsage): TokenUsage {
  if (!isPlainObject(usage)) {
    throw new OpenAIProviderError(options.id, 'usage 必须是普通对象');
  }
  const inputTokens = readCount(options, usage['prompt_tokens'], 'usage.prompt_tokens') ?? 0;
  const outputTokens =
    readCount(options, usage['completion_tokens'], 'usage.completion_tokens') ?? 0;
  const details = usage['prompt_tokens_details'];
  if (details !== undefined && details !== null && !isPlainObject(details)) {
    throw new OpenAIProviderError(options.id, 'usage.prompt_tokens_details 必须是对象');
  }
  const detailsCache =
    details === undefined || details === null
      ? undefined
      : readCount(options, details['cached_tokens'], 'usage.prompt_tokens_details.cached_tokens');
  const deepSeekCache = readCount(
    options,
    usage['prompt_cache_hit_tokens'],
    'usage.prompt_cache_hit_tokens',
  );
  const cacheReadTokens = detailsCache ?? deepSeekCache;

  try {
    return createTokenUsage(
      inputTokens,
      outputTokens,
      cacheReadTokens === undefined ? undefined : { cacheReadTokens },
    );
  } catch (error) {
    throw new OpenAIProviderError(options.id, '返回了非法 usage', { cause: error });
  }
}

/**
 * 映射停止原因。
 *
 * @param reason - 原始 finish_reason
 * @param hasToolCalls - 是否包含工具调用
 * @returns Agent 模型停止原因
 */
export function mapStopReason(reason: string | undefined, hasToolCalls: boolean): ModelStopReason {
  switch (reason) {
    case 'length':
      return ModelStopReason.MaxTokens;
    case 'content_filter':
      return ModelStopReason.ContentFilter;
    case 'tool_calls':
      return ModelStopReason.ToolCalls;
    case 'stop':
      return hasToolCalls ? ModelStopReason.ToolCalls : ModelStopReason.End;
    default:
      return hasToolCalls ? ModelStopReason.ToolCalls : ModelStopReason.Other;
  }
}

/** 读取文本内容。 */
function readText(options: ResolvedOpenAIProviderOptions, message: OpenAIResponseMessage): string {
  const content = message['content'];
  if (content === undefined || content === null) return '';
  if (typeof content !== 'string') {
    throw new OpenAIProviderError(options.id, 'message.content 必须是字符串或 null');
  }
  return content;
}

/** 读取推理内容（字段名可配置）。 */
function readReasoning(
  options: ResolvedOpenAIProviderOptions,
  message: OpenAIResponseMessage,
): string {
  return readOptionalString(
    options,
    message[options.reasoningContentField],
    options.reasoningContentField,
  );
}

/** 读取工具调用数组。 */
function readToolCalls(
  options: ResolvedOpenAIProviderOptions,
  value: readonly OpenAIResponseToolCall[] | undefined,
): AccumulatedToolCall[] {
  if (value === undefined) return [];
  if (!Array.isArray(value)) {
    throw new OpenAIProviderError(options.id, 'message.tool_calls 必须是数组');
  }
  return value.map((call, index) => {
    if (!isPlainObject(call)) {
      throw new OpenAIProviderError(options.id, `message.tool_calls[${String(index)}] 必须是对象`);
    }
    const responseCall = call as OpenAIResponseToolCall;
    const id = readOptionalString(options, responseCall['id'], `tool_calls[${String(index)}].id`);
    const name = readOptionalString(
      options,
      responseCall['function']?.['name'],
      `tool_calls[${String(index)}].name`,
    );
    const argumentsText = readOptionalString(
      options,
      responseCall['function']?.['arguments'],
      `tool_calls[${String(index)}].arguments`,
    );
    if (id === '' || name === '') {
      throw new OpenAIProviderError(
        options.id,
        `message.tool_calls[${String(index)}] 缺少 id 或 function.name`,
      );
    }
    return { id, name, argumentsText };
  });
}

/** 读取并校验 finish_reason。 */
function readFinishReason(
  options: ResolvedOpenAIProviderOptions,
  value: string | null | undefined,
): string | undefined {
  if (value === undefined || value === null) return undefined;
  if (typeof value !== 'string') {
    throw new OpenAIProviderError(options.id, 'finish_reason 必须是字符串或 null');
  }
  return value;
}

/** 读取可选字符串；缺省或 null 视为空字符串。 */
function readOptionalString(
  options: ResolvedOpenAIProviderOptions,
  value: unknown,
  field: string,
): string {
  if (value === undefined || value === null) return '';
  if (typeof value !== 'string') {
    throw new OpenAIProviderError(options.id, `${field} 必须是字符串`);
  }
  return value;
}

/** 读取可选非负安全整数。 */
function readCount(
  options: ResolvedOpenAIProviderOptions,
  value: unknown,
  field: string,
): number | undefined {
  if (value === undefined) return undefined;
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0) {
    throw new OpenAIProviderError(options.id, `${field} 必须是非负安全整数`);
  }
  return value;
}
