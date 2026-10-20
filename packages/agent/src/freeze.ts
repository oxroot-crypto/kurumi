/**
 * @file 事件冻结工具。
 * @description 事件可能包含消息、结果与序列化错误；统一深度冻结，避免观察者互相污染。
 * @module agent/freeze
 */

import { deepFreeze } from '@kurumi/core';

import type { AgentEvent } from './events.js';

/**
 * 深度冻结事件对象。
 *
 * @param event - 待冻结事件
 * @returns 原引用，便于链式返回
 */
export function freezeAgentEvent<TEvent extends AgentEvent>(event: TEvent): TEvent {
  return deepFreeze(event);
}
