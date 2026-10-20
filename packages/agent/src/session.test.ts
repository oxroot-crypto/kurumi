/**
 * @file 进程内会话测试。
 * @description 覆盖初始校验、追加、替换、清空、快照冻结与失败原子性。
 * @module agent/session.test
 */

import { describe, expect, it } from 'vitest';

import { AgentDefinitionError, AgentMessageError } from './errors.js';
import { createAgentSession, createUserMessage, type AgentMessage } from './index.js';

describe('createAgentSession', () => {
  it('空会话返回冻结快照', () => {
    const session = createAgentSession();

    expect(session.messages).toEqual([]);
    expect(Object.isFrozen(session.messages)).toBe(true);
    expect(Object.isFrozen(session)).toBe(true);
  });

  it('校验初始消息并保持输入引用独立', () => {
    const raw = { role: 'user', content: 'hi' } as unknown as AgentMessage;
    const session = createAgentSession({ messages: [raw] });

    expect(session.messages).toEqual([{ role: 'user', content: [{ type: 'text', text: 'hi' }] }]);
    expect(session.messages[0]).not.toBe(raw);
    expect(Object.isFrozen(session.messages[0])).toBe(true);
  });

  it('追加消息生成新快照', () => {
    const session = createAgentSession();
    const first = session.messages;
    session.append(createUserMessage('a'));

    expect(session.messages).not.toBe(first);
    expect(session.messages).toHaveLength(1);
    expect(first).toHaveLength(0);
  });

  it('替换与清空消息', () => {
    const session = createAgentSession({ messages: [createUserMessage('a')] });
    session.replace([createUserMessage('b'), createUserMessage('c')]);

    expect(session.messages.map((message) => message.content[0])).toEqual([
      { type: 'text', text: 'b' },
      { type: 'text', text: 'c' },
    ]);

    session.clear();
    expect(session.messages).toEqual([]);
  });

  it('追加非法消息时保持原状态', () => {
    const session = createAgentSession({ messages: [createUserMessage('a')] });
    const before = session.messages;

    expect(() => session.append({ role: 'user', content: [] } as never)).toThrow(AgentMessageError);
    expect(session.messages).toBe(before);
    expect(session.messages).toHaveLength(1);
  });

  it('替换非法消息时保持原状态', () => {
    const session = createAgentSession({ messages: [createUserMessage('a')] });

    expect(() => session.replace([{ role: 'tool', content: 'x' } as never])).toThrow(
      AgentMessageError,
    );
    expect(session.messages).toHaveLength(1);
  });

  it('拒绝非数组初始消息', () => {
    expect(() => createAgentSession({ messages: null as never })).toThrow(AgentDefinitionError);
  });
});
