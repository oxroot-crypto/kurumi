/**
 * @file Agent 公共配置、运行选项与内部解析后的设置。
 * @description 只包含类型定义，避免实现模块之间产生运行期循环依赖。
 * @module agent/agent-types
 */

import type { JsonValue } from '@kurumi/core';

import type { AgentEvent } from './events.js';
import type { AgentMessage, ToolCallContentPart } from './message.js';
import type { ModelProvider, ResolvedThinkingConfig, ThinkingOption, ToolChoice } from './model.js';
import type { AgentResult } from './result.js';
import type { AgentSession } from './session.js';
import type { ToolDefinition, ToolRegistry } from './tool.js';

/** 默认最大模型轮数。 */
export const DEFAULT_MAX_STEPS = 8;

/** 默认工具并发数。 */
export const DEFAULT_TOOL_CONCURRENCY = 1;

/** 默认工具结果最大字符数。 */
export const DEFAULT_MAX_TOOL_RESULT_LENGTH = 100_000;

/** 默认模型重试次数。 */
export const DEFAULT_MAX_MODEL_RETRIES = 0;

/** 默认模型重试等待时间（毫秒）。 */
export const DEFAULT_RETRY_DELAY_MS = 500;

/** 工具审批上下文。 */
export interface ToolApprovalContext {
  /** 调用发生时的完整对话快照。 */
  readonly messages: readonly AgentMessage[];
  /** 取消信号。 */
  readonly signal: AbortSignal | undefined;
}

/**
 * 工具审批回调。
 *
 * @param call - 模型发起的工具调用
 * @param context - 对话快照与取消信号
 * @returns true 允许执行；false 产生错误结果并回传模型
 */
export type ToolApprovalCallback = (
  call: ToolCallContentPart,
  context: ToolApprovalContext,
) => boolean | Promise<boolean>;

/** Agent 构造配置。 */
export interface AgentConfig {
  /** 模型提供方。 */
  readonly provider: ModelProvider;
  /** 模型名称，非空字符串。 */
  readonly model: string;
  /** 固定 system 提示；若输入消息首条已是 system，则不会重复注入。 */
  readonly system?: string;
  /** 可调用工具；重名会在构造期失败。 */
  readonly tools?: readonly ToolDefinition[];
  /** 最大模型轮数，缺省 8，必须是正整数。 */
  readonly maxSteps?: number;
  /** 工具并发数，缺省 1，必须是正整数。 */
  readonly toolConcurrency?: number;
  /** 工具默认超时（毫秒），缺省不超时。 */
  readonly toolTimeoutMs?: number;
  /** 单条工具结果最大字符数，缺省 100000；超出部分被截断并标记。 */
  readonly maxToolResultLength?: number;
  /** 采样温度，必须是有限数值。 */
  readonly temperature?: number;
  /** 最大输出 Token 数，缺省由提供方决定。 */
  readonly maxOutputTokens?: number;
  /** 工具选择策略。 */
  readonly toolChoice?: ToolChoice;
  /** 思考配置；缺省由提供方默认行为决定。 */
  readonly thinking?: ThinkingOption;
  /** 透传给模型提供方的 JSON 安全选项。 */
  readonly providerOptions?: JsonValue;
  /** 是否优先使用提供方的流式能力，缺省 true。 */
  readonly shouldStream?: boolean;
  /** 模型调用失败后的最大重试次数，缺省 0。 */
  readonly maxModelRetries?: number;
  /** 重试等待时间（毫秒），缺省 500。 */
  readonly retryDelayMs?: number;
  /** 决定某次失败是否重试；缺省全部重试。 */
  readonly shouldRetryModelError?: (error: unknown, attempt: number) => boolean;
  /** 工具审批回调；缺省允许执行。 */
  readonly approveToolCall?: ToolApprovalCallback;
}

/** 单次运行覆盖项。 */
export interface AgentRunOptions {
  /** 取消信号。 */
  readonly signal?: AbortSignal;
  /** 覆盖构造期的最大模型轮数。 */
  readonly maxSteps?: number;
  /** 覆盖构造期的工具选择策略。 */
  readonly toolChoice?: ToolChoice;
  /** 覆盖构造期的思考配置。 */
  readonly thinking?: ThinkingOption;
  /** 覆盖构造期的流式开关。 */
  readonly shouldStream?: boolean;
}

/** 跨 runtime Agent 实例。 */
export interface Agent {
  /** 提供方 ID。 */
  readonly providerId: string;
  /** 模型名称。 */
  readonly model: string;
  /** 工具注册表。 */
  readonly tools: ToolRegistry;
  /**
   * 运行至结束并返回结果。
   *
   * @param messages - 现有对话消息；允许为空但必须配置 system 提示
   * @param options - 单次运行覆盖项
   * @returns 不可变运行结果
   */
  run(messages: readonly AgentMessage[], options?: AgentRunOptions): Promise<AgentResult>;
  /**
   * 以事件流方式运行。
   *
   * @param messages - 现有对话消息
   * @param options - 单次运行覆盖项
   * @returns 事件流；返回值为最终结果
   */
  stream(
    messages: readonly AgentMessage[],
    options?: AgentRunOptions,
  ): AsyncGenerator<AgentEvent, AgentResult, void>;
  /**
   * 在会话中运行，并把本次生成的消息追加到会话。
   *
   * @param session - 目标会话
   * @param options - 单次运行覆盖项
   * @returns 不可变运行结果
   */
  runInSession(session: AgentSession, options?: AgentRunOptions): Promise<AgentResult>;
}

/** 内部解析后的 Agent 设置。 */
export interface AgentSettings {
  /** 模型提供方。 */
  readonly provider: ModelProvider;
  /** 提供方 ID。 */
  readonly providerId: string;
  /** 模型名称。 */
  readonly model: string;
  /** 固定 system 提示。 */
  readonly system: string | undefined;
  /** 工具注册表。 */
  readonly registry: ToolRegistry;
  /** 最大模型轮数。 */
  readonly maxSteps: number;
  /** 工具并发数。 */
  readonly toolConcurrency: number;
  /** 工具默认超时。 */
  readonly toolTimeoutMs: number | undefined;
  /** 单条工具结果最大字符数。 */
  readonly maxToolResultLength: number;
  /** 采样温度。 */
  readonly temperature: number | undefined;
  /** 最大输出 Token 数。 */
  readonly maxOutputTokens: number | undefined;
  /** 归一化后的思考配置。 */
  readonly thinking: ResolvedThinkingConfig | undefined;
  /** 透传选项。 */
  readonly providerOptions: JsonValue | undefined;
  /** 是否优先流式。 */
  readonly shouldStream: boolean;
  /** 模型重试次数。 */
  readonly maxModelRetries: number;
  /** 重试等待时间。 */
  readonly retryDelayMs: number;
  /** 重试判定。 */
  readonly shouldRetryModelError: (error: unknown, attempt: number) => boolean;
  /** 工具审批。 */
  readonly approveToolCall: ToolApprovalCallback | undefined;
  /** 工具选择策略。 */
  readonly toolChoice: ToolChoice | undefined;
}

/** 内部解析后的单次运行设置。 */
export interface PreparedRun {
  /** 注入 system 后的冻结消息快照。 */
  readonly messages: readonly AgentMessage[];
  /** 生效的最大轮数。 */
  readonly maxSteps: number;
  /** 生效的工具选择策略。 */
  readonly toolChoice: ToolChoice | undefined;
  /** 生效的思考配置。 */
  readonly thinking: ResolvedThinkingConfig | undefined;
  /** 生效的流式开关。 */
  readonly shouldStream: boolean;
  /** 取消信号。 */
  readonly signal: AbortSignal | undefined;
}
