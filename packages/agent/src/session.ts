/**
 * @file 进程内 Agent 会话。
 * @description 只维护消息数组与不可变快照；持久化与并发控制由调用方负责。
 * @module agent/session
 */

import { AgentDefinitionError } from './errors.js';
import { normalizeAgentMessage, type AgentMessage } from './message.js';

/** `createAgentSession` 的选项。 */
export interface CreateAgentSessionOptions {
  /** 初始消息；会逐条校验并冻结。 */
  readonly messages?: readonly AgentMessage[];
}

/** 可变的进程内会话；`messages` 每次读取都返回当前冻结快照。 */
export interface AgentSession {
  /** 当前消息快照。 */
  readonly messages: readonly AgentMessage[];
  /**
   * 追加消息。
   *
   * @param messages - 待追加消息；会逐条校验
   * @throws {AgentMessageError} 消息结构非法
   */
  append(...messages: AgentMessage[]): void;
  /**
   * 替换全部消息。
   *
   * @param messages - 新消息数组；会逐条校验
   * @throws {AgentMessageError} 消息结构非法
   */
  replace(messages: readonly AgentMessage[]): void;
  /** 清空会话。 */
  clear(): void;
}

/**
 * 创建进程内会话。
 *
 * @param options - 可选初始消息
 * @returns 冻结的会话对象；内部消息数组可变但每次读取都是冻结快照
 * @throws {AgentDefinitionError} `messages` 不是数组
 * @throws {AgentMessageError} 初始消息非法
 *
 * @remarks 会话不提供并发保护；同一会话上的并行运行可能产生重复或交错消息。
 *
 * @example
 * ```ts
 * const session = createAgentSession({ messages: [createUserMessage('你好')] });
 * session.append(createUserMessage('继续'));
 * ```
 */
export function createAgentSession(options: CreateAgentSessionOptions = {}): AgentSession {
  if (typeof options !== 'object' || options === null) {
    throw new AgentDefinitionError('createAgentSession options 必须是对象');
  }
  const initial = options.messages === undefined ? [] : options.messages;
  let messages = normalizeMessages(initial, '$.messages');

  return Object.freeze({
    get messages(): readonly AgentMessage[] {
      return messages;
    },
    append(...next: AgentMessage[]): void {
      messages = Object.freeze([...messages, ...normalizeMessages(next, '$.messages')]);
    },
    replace(next: readonly AgentMessage[]): void {
      messages = normalizeMessages(next, '$.messages');
    },
    clear(): void {
      messages = Object.freeze([]);
    },
  });
}

/** 校验消息数组并返回冻结快照。 */
function normalizeMessages(
  messages: readonly AgentMessage[],
  path: string,
): readonly AgentMessage[] {
  if (!Array.isArray(messages)) {
    throw new AgentDefinitionError(`${path} 必须是消息数组`);
  }
  return Object.freeze(
    messages.map((message, index) => normalizeAgentMessage(message, `${path}[${index}]`)),
  );
}
