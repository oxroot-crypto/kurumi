/**
 * @file Agent 运行结果与停止原因。
 * @description 结果是完整对话的快照；调用方可直接用于下一轮运行或持久化。
 * @module agent/result
 */

import type { AgentMessage } from './message.js';
import type { ModelStopReason } from './model.js';
import type { TokenUsage } from './usage.js';

/** Agent 停止原因。 */
export const AgentStopReason = {
  /** 模型给出了最终文本响应，无需继续调用工具。 */
  Completed: 'completed',
  /** 达到 `maxSteps` 上限。 */
  MaxSteps: 'max_steps',
  /** 调用方通过 AbortSignal 取消。 */
  Aborted: 'aborted',
} as const;

/** Agent 停止原因联合类型。 */
export type AgentStopReason = (typeof AgentStopReason)[keyof typeof AgentStopReason];

/** 一次 Agent 运行的不可变结果。 */
export interface AgentResult {
  /** 完整对话快照：输入消息 + 本次生成的全部消息。 */
  readonly messages: readonly AgentMessage[];
  /** 本次运行新增的消息，按生成顺序排列。 */
  readonly generatedMessages: readonly AgentMessage[];
  /** 最终 assistant 消息；仅当正常完成且无工具调用时存在。 */
  readonly finalMessage: AgentMessage | undefined;
  /** 实际完成的模型调用轮数。 */
  readonly steps: number;
  /** 本次运行的累计 Token 用量。 */
  readonly usage: TokenUsage;
  /** Agent 级停止原因。 */
  readonly stopReason: AgentStopReason;
  /** 最后一次模型响应的停止原因；未收到响应时为 undefined。 */
  readonly modelStopReason: ModelStopReason | undefined;
  /** 因失败而重试模型调用的次数。 */
  readonly retryCount: number;
}
