/**
 * @file Agent 工厂与一次性运行入口。
 * @description 配置解析、运行循环与工具执行分别位于 agent-settings/agent-run/tool-execution。
 * @module agent/agent
 */

import { prepareRun, resolveAgentSettings } from './agent-settings.js';
import { runPrepared } from './agent-run.js';
import type { Agent, AgentConfig, AgentRunOptions } from './agent-types.js';
import type { AgentEvent } from './events.js';
import type { AgentMessage } from './message.js';
import type { AgentResult } from './result.js';
import type { AgentSession } from './session.js';

/**
 * 创建可复用 Agent 实例。
 *
 * @param config - 提供方、模型、工具与运行上限
 * @returns 冻结的 Agent 实例
 * @throws {AgentDefinitionError} 配置、工具定义或工具选择非法
 *
 * @example
 * ```ts
 * const agent = createAgent({ provider: openaiProvider, model: 'gpt-4.1-mini', tools: [search] });
 * const result = await agent.run([createUserMessage('你好')]);
 * ```
 */
export function createAgent(config: AgentConfig): Agent {
  const settings = resolveAgentSettings(config);
  const stream = (messages: readonly AgentMessage[], options?: AgentRunOptions) =>
    runPrepared(settings, prepareRun(settings, messages, options));
  const run = async (messages: readonly AgentMessage[], options?: AgentRunOptions) =>
    collectResult(stream(messages, options));
  const runInSession = async (session: AgentSession, options?: AgentRunOptions) => {
    const result = await run(session.messages, options);
    session.append(...result.generatedMessages);
    return result;
  };

  return Object.freeze({
    providerId: settings.providerId,
    model: settings.model,
    tools: settings.registry,
    run,
    stream,
    runInSession,
  });
}

/**
 * 一次性创建并运行 Agent。
 *
 * @param config - Agent 配置
 * @param messages - 现有对话消息
 * @param options - 单次运行覆盖项
 * @returns 不可变运行结果
 */
export function runAgent(
  config: AgentConfig,
  messages: readonly AgentMessage[],
  options?: AgentRunOptions,
): Promise<AgentResult> {
  return createAgent(config).run(messages, options);
}

/**
 * 一次性创建并以事件流运行 Agent。
 *
 * @param config - Agent 配置
 * @param messages - 现有对话消息
 * @param options - 单次运行覆盖项
 * @returns 事件流；返回值为最终结果
 */
export function streamAgent(
  config: AgentConfig,
  messages: readonly AgentMessage[],
  options?: AgentRunOptions,
): AsyncGenerator<AgentEvent, AgentResult, void> {
  return createAgent(config).stream(messages, options);
}

/** 消费事件流并返回最终结果。 */
async function collectResult(
  stream: AsyncGenerator<AgentEvent, AgentResult, void>,
): Promise<AgentResult> {
  let next = await stream.next();
  while (next.done !== true) next = await stream.next();
  return next.value;
}
