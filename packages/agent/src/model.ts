/**
 * @file 模型提供方契约、停止原因、流式增量与响应校验。
 * @description 模型适配器只依赖本文件的公开类型；Agent 循环负责运行期校验。
 * @module agent/model
 */

import { cloneJsonValue, deepFreeze, isPlainObject, type JsonValue } from '@kurumi/core';

import { AgentDefinitionError, AgentModelError } from './errors.js';
import {
  AgentMessageRole,
  isValidToolName,
  normalizeAgentMessage,
  type AgentMessage,
} from './message.js';
import type { ToolDescriptor } from './tool.js';
import { EMPTY_TOKEN_USAGE, normalizeTokenUsage, type TokenUsage } from './usage.js';

/** 模型停止原因。 */
export const ModelStopReason = {
  /** 模型正常结束。 */
  End: 'end',
  /** 达到最大输出 Token。 */
  MaxTokens: 'max_tokens',
  /** 模型请求调用工具。 */
  ToolCalls: 'tool_calls',
  /** 内容被安全策略拦截。 */
  ContentFilter: 'content_filter',
  /** 提供方返回了无法归类的停止原因。 */
  Other: 'other',
} as const;

/** 模型停止原因联合类型。 */
export type ModelStopReason = (typeof ModelStopReason)[keyof typeof ModelStopReason];

/** 工具选择策略。 */
export type ToolChoice =
  'auto' | 'none' | 'required' | { readonly type: 'tool'; readonly name: string };

/** 推理强度档位；具体语义由提供方解释。 */
export const ThinkingEffort = {
  /** 最小推理量。 */
  Minimal: 'minimal',
  /** 低推理量。 */
  Low: 'low',
  /** 中等推理量。 */
  Medium: 'medium',
  /** 高推理量。 */
  High: 'high',
} as const;

/** 推理强度联合类型。 */
export type ThinkingEffort = (typeof ThinkingEffort)[keyof typeof ThinkingEffort];

/** 思考配置输入；`isEnabled` 缺省为 true。 */
export interface ThinkingConfig {
  /** 是否启用思考；false 显式关闭。 */
  readonly isEnabled?: boolean;
  /** 思考预算 Token 数，正安全整数；由支持预算的提供方解释。 */
  readonly budgetTokens?: number;
  /** 推理强度；由支持 effort 的提供方解释。 */
  readonly effort?: ThinkingEffort;
}

/** 归一化后的思考配置；`isEnabled` 始终存在且对象已冻结。 */
export interface ResolvedThinkingConfig {
  /** 是否启用思考。 */
  readonly isEnabled: boolean;
  /** 思考预算 Token 数。 */
  readonly budgetTokens?: number;
  /** 推理强度。 */
  readonly effort?: ThinkingEffort;
}

/** Agent/运行选项接受的思考配置；布尔值是 `{ isEnabled }` 的简写。 */
export type ThinkingOption = boolean | ThinkingConfig;

/** 一次模型调用的输入。 */
export interface ModelRequest {
  /** 模型名称。 */
  readonly model: string;
  /** 完整、冻结的对话消息。 */
  readonly messages: readonly AgentMessage[];
  /** 可供模型调用的工具描述。 */
  readonly tools: readonly ToolDescriptor[];
  /** 工具选择策略。 */
  readonly toolChoice?: ToolChoice;
  /** 采样温度，由适配器决定合法区间。 */
  readonly temperature?: number;
  /** 最大输出 Token 数。 */
  readonly maxOutputTokens?: number;
  /** 归一化后的思考配置；缺省表示由提供方默认行为决定。 */
  readonly thinking?: ResolvedThinkingConfig;
  /** 透传给提供方的 JSON 安全选项。 */
  readonly providerOptions?: JsonValue;
}

/** 模型调用选项。 */
export interface ModelCallOptions {
  /** 取消信号。 */
  readonly signal?: AbortSignal;
}

/** 模型响应。 */
export interface ModelResponse {
  /** 必须为 assistant 角色的消息。 */
  readonly message: AgentMessage;
  /** 停止原因。 */
  readonly stopReason: ModelStopReason;
  /** Token 用量；缺省视为零。 */
  readonly usage?: TokenUsage;
}

/** 文本增量。 */
export interface ModelTextDelta {
  /** 增量判别字段。 */
  readonly type: 'text_delta';
  /** 新增文本。 */
  readonly text: string;
}

/** 推理文本增量。 */
export interface ModelReasoningDelta {
  /** 增量判别字段。 */
  readonly type: 'reasoning_delta';
  /** 新增推理文本。 */
  readonly text: string;
}

/** 工具调用参数增量。 */
export interface ModelToolCallDelta {
  /** 增量判别字段。 */
  readonly type: 'tool_call_delta';
  /** 工具调用在本次响应中的序号，非负安全整数。 */
  readonly index: number;
  /** 首次出现时可携带调用 ID。 */
  readonly callId?: string;
  /** 首次出现时可携带工具名称。 */
  readonly name?: string;
  /** 原始 JSON 参数片段，由适配器保证可拼接。 */
  readonly argumentsDelta: string;
}

/** 流式增量联合类型。 */
export type ModelDelta = ModelTextDelta | ModelReasoningDelta | ModelToolCallDelta;

/** 流式响应的最终事件；必须是流的最后一个事件且恰好出现一次。 */
export interface ModelResponseEvent {
  /** 事件判别字段。 */
  readonly type: 'response';
  /** 完整响应。 */
  readonly response: ModelResponse;
}

/** 模型流事件联合类型。 */
export type ModelStreamEvent = ModelDelta | ModelResponseEvent;

/**
 * 模型提供方。
 *
 * @remarks
 * - `generate` 是必选能力，适用于所有适配器。
 * - `stream` 为可选增量能力；提供时 Agent 默认优先使用，最终仍必须以 `response` 事件返回完整响应。
 * - 适配器不得修改请求中的消息或工具数组；它们已递归冻结。
 */
export interface ModelProvider {
  /** 提供方唯一 ID，例如 `openai`。 */
  readonly id: string;
  /**
   * 非流式生成。
   *
   * @param request - 冻结的模型请求
   * @param options - 取消信号
   * @returns 完整模型响应
   */
  generate(request: ModelRequest, options: ModelCallOptions): Promise<ModelResponse>;
  /**
   * 流式生成。
   *
   * @param request - 冻结的模型请求
   * @param options - 取消信号
   * @returns 增量事件流；最后一个事件必须是 `response`
   */
  stream?(request: ModelRequest, options: ModelCallOptions): AsyncIterable<ModelStreamEvent>;
}

/** 归一化后的模型响应；`usage` 始终存在。 */
export interface ResolvedModelResponse {
  /** 已校验、冻结的 assistant 消息。 */
  readonly message: AgentMessage;
  /** 已校验的停止原因。 */
  readonly stopReason: ModelStopReason;
  /** 非空用量对象。 */
  readonly usage: TokenUsage;
}

/**
 * 判断任意值是否为合法停止原因。
 *
 * @param value - 待判断值
 * @returns 是 `ModelStopReason` 成员时返回 true
 */
export function isModelStopReason(value: unknown): value is ModelStopReason {
  return (
    value === ModelStopReason.End ||
    value === ModelStopReason.MaxTokens ||
    value === ModelStopReason.ToolCalls ||
    value === ModelStopReason.ContentFilter ||
    value === ModelStopReason.Other
  );
}

/**
 * 校验并归一化工具选择策略。
 *
 * @param value - 未经信任的策略值
 * @param path - 出错路径
 * @returns 冻结的策略
 * @throws {AgentDefinitionError} 字符串或对象结构非法
 */
export function normalizeToolChoice(value: unknown, path = '$.toolChoice'): ToolChoice {
  if (value === 'auto' || value === 'none' || value === 'required') return value;
  if (isPlainObject(value) && value['type'] === 'tool') {
    const name = value['name'];
    if (!isValidToolName(name)) {
      throw new AgentDefinitionError(`${path}.name 必须匹配工具名称规则`);
    }
    return Object.freeze({ type: 'tool', name });
  }
  throw new AgentDefinitionError(`${path} 必须是 auto/none/required 或 { type: "tool", name }`);
}

/**
 * 判断任意值是否为合法推理强度。
 *
 * @param value - 待判断值
 * @returns 是 `ThinkingEffort` 成员时返回 true
 */
export function isThinkingEffort(value: unknown): value is ThinkingEffort {
  return (
    value === ThinkingEffort.Minimal ||
    value === ThinkingEffort.Low ||
    value === ThinkingEffort.Medium ||
    value === ThinkingEffort.High
  );
}

/**
 * 校验并归一化思考配置。
 *
 * @param value - 未经信任的配置值；布尔值表示启用/关闭
 * @param path - 出错路径
 * @returns 冻结的归一化配置
 * @throws {AgentDefinitionError} 值不是布尔/对象，或字段非法
 *
 * @example
 * ```ts
 * normalizeThinkingConfig({ budgetTokens: 2048, effort: 'high' });
 * // { isEnabled: true, budgetTokens: 2048, effort: 'high' }
 * ```
 */
export function normalizeThinkingConfig(
  value: unknown,
  path = '$.thinking',
): ResolvedThinkingConfig {
  if (typeof value === 'boolean') return Object.freeze({ isEnabled: value });
  if (!isPlainObject(value)) {
    throw new AgentDefinitionError(`${path} 必须是布尔值或普通对象`);
  }
  const isEnabled = value['isEnabled'] === undefined ? true : value['isEnabled'];
  if (typeof isEnabled !== 'boolean') {
    throw new AgentDefinitionError(`${path}.isEnabled 必须是布尔值`);
  }
  const budgetTokens = value['budgetTokens'];
  if (
    budgetTokens !== undefined &&
    (typeof budgetTokens !== 'number' || !Number.isSafeInteger(budgetTokens) || budgetTokens <= 0)
  ) {
    throw new AgentDefinitionError(`${path}.budgetTokens 必须是正安全整数`);
  }
  const effort = value['effort'];
  if (effort !== undefined && !isThinkingEffort(effort)) {
    throw new AgentDefinitionError(`${path}.effort 必须是 minimal/low/medium/high 之一`);
  }
  return Object.freeze({
    isEnabled,
    ...(budgetTokens === undefined ? {} : { budgetTokens }),
    ...(effort === undefined ? {} : { effort }),
  });
}

/**
 * 克隆并冻结透传选项。
 *
 * @param value - 未经信任的 JSON 值
 * @param path - 出错路径
 * @returns 冻结的 JSON 值
 * @throws {AgentDefinitionError} 值不是 JSON 安全结构
 */
export function normalizeProviderOptions(value: unknown, path = '$.providerOptions'): JsonValue {
  try {
    return deepFreeze(cloneJsonValue(value, path));
  } catch (error) {
    throw new AgentDefinitionError(`${path} 必须是 JSON 安全值`, error);
  }
}

/**
 * 校验并归一化模型响应。
 *
 * @param providerId - 提供方 ID，用于错误上下文
 * @param value - 未经信任的响应
 * @returns 冻结且 `usage` 必定存在的响应
 * @throws {AgentModelError} 消息角色/结构、停止原因或 usage 非法
 */
export function normalizeModelResponse(providerId: string, value: unknown): ResolvedModelResponse {
  if (!isPlainObject(value)) {
    throw new AgentModelError(providerId, '返回的响应必须是普通对象');
  }
  const message = normalizeResponseMessage(providerId, value['message']);
  const stopReason = value['stopReason'];
  if (!isModelStopReason(stopReason)) {
    throw new AgentModelError(providerId, '返回了非法 stopReason');
  }
  const usage = normalizeResponseUsage(providerId, value['usage']);
  return Object.freeze({ message, stopReason, usage });
}

/**
 * 校验并归一化流事件。
 *
 * @param providerId - 提供方 ID，用于错误上下文
 * @param value - 未经信任的流事件
 * @returns 冻结的流事件
 * @throws {AgentModelError} 增量字段或最终响应非法
 */
export function normalizeModelStreamEvent(providerId: string, value: unknown): ModelStreamEvent {
  if (!isPlainObject(value)) {
    throw new AgentModelError(providerId, '返回了非法流事件');
  }
  switch (value['type']) {
    case 'text_delta':
      return normalizeTextDelta(providerId, value['text']);
    case 'reasoning_delta':
      return normalizeReasoningDelta(providerId, value['text']);
    case 'tool_call_delta':
      return normalizeToolCallDelta(providerId, value);
    case 'response':
      return Object.freeze({
        type: 'response',
        response: normalizeModelResponse(providerId, value['response']),
      });
    default:
      throw new AgentModelError(providerId, '返回了未知流事件类型');
  }
}

/** 校验响应消息。 */
function normalizeResponseMessage(providerId: string, value: unknown): AgentMessage {
  let message: AgentMessage;
  try {
    message = normalizeAgentMessage(value, '$.response.message');
  } catch (error) {
    throw new AgentModelError(providerId, '返回了非法消息', error);
  }
  if (message.role !== AgentMessageRole.Assistant) {
    throw new AgentModelError(providerId, '返回的消息角色必须是 assistant');
  }
  return message;
}

/** 校验响应 usage。 */
function normalizeResponseUsage(providerId: string, value: unknown): TokenUsage {
  if (value === undefined) return EMPTY_TOKEN_USAGE;
  try {
    return normalizeTokenUsage(value, '$.response.usage');
  } catch (error) {
    throw new AgentModelError(providerId, '返回了非法 usage', error);
  }
}

/** 校验文本增量。 */
function normalizeTextDelta(providerId: string, text: unknown): ModelTextDelta {
  if (typeof text !== 'string') {
    throw new AgentModelError(providerId, 'text_delta.text 必须是字符串');
  }
  return Object.freeze({ type: 'text_delta', text });
}

/** 校验推理增量。 */
function normalizeReasoningDelta(providerId: string, text: unknown): ModelReasoningDelta {
  if (typeof text !== 'string') {
    throw new AgentModelError(providerId, 'reasoning_delta.text 必须是字符串');
  }
  return Object.freeze({ type: 'reasoning_delta', text });
}

/** 校验工具调用参数增量。 */
function normalizeToolCallDelta(
  providerId: string,
  value: Record<string, unknown>,
): ModelToolCallDelta {
  const index = value['index'];
  if (typeof index !== 'number' || !Number.isSafeInteger(index) || index < 0) {
    throw new AgentModelError(providerId, 'tool_call_delta.index 必须是非负安全整数');
  }
  const argumentsDelta = value['argumentsDelta'];
  if (typeof argumentsDelta !== 'string') {
    throw new AgentModelError(providerId, 'tool_call_delta.argumentsDelta 必须是字符串');
  }
  const callId = value['callId'];
  if (callId !== undefined && (typeof callId !== 'string' || callId.trim() === '')) {
    throw new AgentModelError(providerId, 'tool_call_delta.callId 必须是非空字符串');
  }
  const name = value['name'];
  if (name !== undefined && !isValidToolName(name)) {
    throw new AgentModelError(providerId, 'tool_call_delta.name 必须匹配工具名称规则');
  }
  return Object.freeze({
    type: 'tool_call_delta',
    index,
    ...(callId === undefined ? {} : { callId }),
    ...(name === undefined ? {} : { name }),
    argumentsDelta,
  });
}
