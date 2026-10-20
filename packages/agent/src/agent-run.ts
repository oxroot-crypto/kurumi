/**
 * @file Agent 运行循环。
 * @description 组合模型调用、工具执行、取消与轮数上限，产出确定性事件流。
 * @module agent/agent-run
 */

import { isAborted } from './abort.js';
import type { AgentSettings, PreparedRun } from './agent-types.js';
import { AgentDefinitionError, AgentModelError } from './errors.js';
import type { AgentEvent } from './events.js';
import { freezeAgentEvent } from './freeze.js';
import {
  createToolMessage,
  getToolCalls,
  type AgentMessage,
  type ToolCallContentPart,
} from './message.js';
import { ModelStopReason } from './model.js';
import { callModel } from './model-call.js';
import { AgentStopReason, type AgentResult } from './result.js';
import { executeToolCalls } from './tool-execution.js';
import type { ToolResult } from './tool.js';
import { EMPTY_TOKEN_USAGE, addTokenUsage, type TokenUsage } from './usage.js';

/** 内部运行状态。 */
interface RunState {
  /** 会话消息的可变工作副本。 */
  readonly messages: AgentMessage[];
  /** 本次生成消息的可变工作副本。 */
  readonly generatedMessages: AgentMessage[];
  /** 已完成的模型轮数。 */
  step: number;
  /** 累计用量。 */
  usage: TokenUsage;
  /** 最终 assistant 消息。 */
  finalMessage: AgentMessage | undefined;
  /** 最后一次模型停止原因。 */
  modelStopReason: ModelStopReason | undefined;
  /** 累计重试次数。 */
  retryCount: number;
}

/**
 * 执行运行循环。
 *
 * @param settings - 已解析的 Agent 设置
 * @param prepared - 单次运行快照
 * @returns 事件流；返回最终结果
 */
export async function* runPrepared(
  settings: AgentSettings,
  prepared: PreparedRun,
): AsyncGenerator<AgentEvent, AgentResult, void> {
  const state = createRunState(prepared);
  yield freezeAgentEvent({
    type: 'agent_start',
    providerId: settings.providerId,
    model: settings.model,
    messages: Object.freeze([...state.messages]),
  });

  while (state.step < prepared.maxSteps) {
    if (isAborted(prepared.signal)) return yield* endWith(state, AgentStopReason.Aborted);

    const step = state.step + 1;
    yield freezeAgentEvent({ type: 'step_start', step });
    const outcome = yield* callModel({
      settings,
      prepared,
      messages: state.messages,
      step,
    });
    state.retryCount += outcome.retryCount;
    if (outcome.response === undefined || isAborted(prepared.signal)) {
      return yield* endWith(state, AgentStopReason.Aborted);
    }

    const response = outcome.response;
    state.step = step;
    state.usage = addTokenUsage(state.usage, response.usage);
    state.modelStopReason = response.stopReason;
    state.messages.push(response.message);
    state.generatedMessages.push(response.message);
    yield freezeAgentEvent({
      type: 'assistant_message',
      step,
      message: response.message,
      stopReason: response.stopReason,
      usage: response.usage,
    });

    const calls = getToolCalls(response.message);
    if (calls.length === 0) {
      if (response.stopReason === ModelStopReason.ToolCalls) {
        throw new AgentModelError(
          settings.providerId,
          '返回 stopReason=tool_calls 但消息中没有工具调用',
        );
      }
      state.finalMessage = response.message;
      yield freezeAgentEvent({
        type: 'step_end',
        step,
        stopReason: response.stopReason,
      });
      return yield* endWith(state, AgentStopReason.Completed);
    }

    const results = yield* executeToolCalls({
      calls,
      settings,
      messages: Object.freeze([...state.messages]),
      signal: prepared.signal,
      step,
    });
    appendToolMessages(state, calls, results);
    yield freezeAgentEvent({
      type: 'step_end',
      step,
      stopReason: response.stopReason,
    });
    if (isAborted(prepared.signal)) {
      return yield* endWith(state, AgentStopReason.Aborted);
    }
  }

  return yield* endWith(state, AgentStopReason.MaxSteps);
}

/** 输出结束事件并返回结果。 */
function* endWith(
  state: RunState,
  stopReason: AgentStopReason,
): Generator<AgentEvent, AgentResult, void> {
  const result = createAgentResult(state, stopReason);
  yield freezeAgentEvent({ type: 'agent_end', result });
  return result;
}

/** 构造初始运行状态。 */
function createRunState(prepared: PreparedRun): RunState {
  return {
    messages: [...prepared.messages],
    generatedMessages: [],
    step: 0,
    usage: EMPTY_TOKEN_USAGE,
    finalMessage: undefined,
    modelStopReason: undefined,
    retryCount: 0,
  };
}

/** 构造冻结结果。 */
function createAgentResult(state: RunState, stopReason: AgentStopReason): AgentResult {
  return Object.freeze({
    messages: Object.freeze([...state.messages]),
    generatedMessages: Object.freeze([...state.generatedMessages]),
    finalMessage: state.finalMessage,
    steps: state.step,
    usage: state.usage,
    stopReason,
    modelStopReason: state.modelStopReason,
    retryCount: state.retryCount,
  });
}

/** 按调用顺序追加工具结果消息。 */
function appendToolMessages(
  state: RunState,
  calls: readonly ToolCallContentPart[],
  results: readonly ToolResult[],
): void {
  calls.forEach((call, index) => {
    const result = results[index];
    if (result === undefined) {
      throw new AgentDefinitionError(`工具 "${call.name}" 缺少执行结果`);
    }
    const message = createToolMessage({
      callId: call.callId,
      name: call.name,
      content: result.content,
      isError: result.isError,
      ...(result.metadata === undefined ? {} : { metadata: result.metadata }),
    });
    state.messages.push(message);
    state.generatedMessages.push(message);
  });
}
