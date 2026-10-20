/**
 * @file Agent 运行事件。
 * @description 事件流是 SDK 唯一的观测接口；不写日志、不依赖 console。
 * @module agent/events
 */

import type { JsonValue, SerializedError } from '@kurumi/core';

import type { AgentMessage } from './message.js';
import type { ModelDelta, ModelStopReason } from './model.js';
import type { AgentResult } from './result.js';
import type { ToolResult } from './tool.js';
import type { TokenUsage } from './usage.js';

/** Agent 事件类型。 */
export const AgentEventType = {
  /** 运行开始。 */
  Start: 'agent_start',
  /** 一轮模型调用开始。 */
  StepStart: 'step_start',
  /** 模型流式增量。 */
  ModelDelta: 'model_delta',
  /** 模型调用即将重试。 */
  ModelRetry: 'model_retry',
  /** 收到完整 assistant 消息。 */
  AssistantMessage: 'assistant_message',
  /** 工具调用开始。 */
  ToolStart: 'tool_start',
  /** 工具调用结束。 */
  ToolEnd: 'tool_end',
  /** 一轮模型调用结束。 */
  StepEnd: 'step_end',
  /** 运行结束，携带最终结果。 */
  End: 'agent_end',
} as const;

/** Agent 事件类型联合。 */
export type AgentEventType = (typeof AgentEventType)[keyof typeof AgentEventType];

/** 运行开始事件。 */
export interface AgentStartEvent {
  /** 事件判别字段。 */
  readonly type: 'agent_start';
  /** 运行时的提供方 ID。 */
  readonly providerId: string;
  /** 运行时的模型名称。 */
  readonly model: string;
  /** 注入 system 提示后的初始消息快照。 */
  readonly messages: readonly AgentMessage[];
}

/** 单轮模型调用开始事件。 */
export interface AgentStepStartEvent {
  /** 事件判别字段。 */
  readonly type: 'step_start';
  /** 从 1 开始的轮次编号。 */
  readonly step: number;
}

/** 模型流式增量事件。 */
export interface AgentModelDeltaEvent {
  /** 事件判别字段。 */
  readonly type: 'model_delta';
  /** 当前轮次编号。 */
  readonly step: number;
  /** 模型增量。 */
  readonly delta: ModelDelta;
}

/** 模型调用重试事件；此前的增量应被视为废弃。 */
export interface AgentModelRetryEvent {
  /** 事件判别字段。 */
  readonly type: 'model_retry';
  /** 当前轮次编号。 */
  readonly step: number;
  /** 即将进行的重试次数，从 1 开始。 */
  readonly attempt: number;
  /** 重试前等待时间（毫秒）。 */
  readonly delayMs: number;
  /** 已序列化的失败原因。 */
  readonly error: SerializedError;
}

/** 完整 assistant 消息事件。 */
export interface AgentAssistantMessageEvent {
  /** 事件判别字段。 */
  readonly type: 'assistant_message';
  /** 当前轮次编号。 */
  readonly step: number;
  /** 已校验并冻结的消息。 */
  readonly message: AgentMessage;
  /** 模型停止原因。 */
  readonly stopReason: ModelStopReason;
  /** 本轮 Token 用量。 */
  readonly usage: TokenUsage;
}

/** 工具调用开始事件。 */
export interface AgentToolStartEvent {
  /** 事件判别字段。 */
  readonly type: 'tool_start';
  /** 当前轮次编号。 */
  readonly step: number;
  /** 模型提供的调用 ID。 */
  readonly callId: string;
  /** 工具名称。 */
  readonly name: string;
  /** 工具输入的 JSON 快照。 */
  readonly input: JsonValue;
}

/** 工具调用结束事件。 */
export interface AgentToolEndEvent {
  /** 事件判别字段。 */
  readonly type: 'tool_end';
  /** 当前轮次编号。 */
  readonly step: number;
  /** 模型提供的调用 ID。 */
  readonly callId: string;
  /** 工具名称。 */
  readonly name: string;
  /** 归一化后的工具结果。 */
  readonly result: ToolResult;
}

/** 单轮模型调用结束事件。 */
export interface AgentStepEndEvent {
  /** 事件判别字段。 */
  readonly type: 'step_end';
  /** 当前轮次编号。 */
  readonly step: number;
  /** 模型停止原因。 */
  readonly stopReason: ModelStopReason;
}

/** 运行结束事件。 */
export interface AgentEndEvent {
  /** 事件判别字段。 */
  readonly type: 'agent_end';
  /** 不可变运行结果。 */
  readonly result: AgentResult;
}

/** Agent 事件联合类型。 */
export type AgentEvent =
  | AgentStartEvent
  | AgentStepStartEvent
  | AgentModelDeltaEvent
  | AgentModelRetryEvent
  | AgentAssistantMessageEvent
  | AgentToolStartEvent
  | AgentToolEndEvent
  | AgentStepEndEvent
  | AgentEndEvent;
