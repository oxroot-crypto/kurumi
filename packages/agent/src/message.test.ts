/**
 * @file 消息、内容块与角色/内容矩阵测试。
 * @description 覆盖创建、归一化、冻结、工具调用提取与非法结构。
 * @module agent/message.test
 */

import { describe, expect, it } from 'vitest';

import type { JsonValue } from '@kurumi/core';

import { AgentMessageError } from './errors.js';
import {
  AgentMessageRole,
  createAssistantMessage,
  createMessage,
  createSystemMessage,
  createToolMessage,
  createUserMessage,
  extractMessageReasoning,
  extractMessageText,
  getToolCalls,
  isValidToolName,
  normalizeAgentMessage,
} from './index.js';

describe('消息创建', () => {
  it('字符串简写生成 text 内容块', () => {
    const message = createUserMessage('你好');

    expect(message.role).toBe(AgentMessageRole.User);
    expect(message.content).toEqual([{ type: 'text', text: '你好' }]);
    expect(Object.isFrozen(message)).toBe(true);
    expect(Object.isFrozen(message.content)).toBe(true);
    expect(Object.isFrozen(message.content[0])).toBe(true);
  });

  it('支持 system / user / assistant 角色', () => {
    expect(createSystemMessage('规则').role).toBe('system');
    expect(createUserMessage('问题').role).toBe('user');
    expect(createAssistantMessage('回答').role).toBe('assistant');
  });

  it('user 消息支持图片内容块', () => {
    const message = createUserMessage([
      { type: 'text', text: '看图' },
      { type: 'image', mediaType: 'image/png', data: 'aGVsbG8=' },
    ]);

    expect(message.content).toHaveLength(2);
    expect(message.content[1]).toEqual({
      type: 'image',
      mediaType: 'image/png',
      data: 'aGVsbG8=',
    });
  });

  it('assistant 消息支持 reasoning 与 tool_call', () => {
    const message = createAssistantMessage([
      { type: 'reasoning', text: '思考' },
      { type: 'tool_call', callId: 'call-1', name: 'search', input: { query: 'x' } },
      { type: 'text', text: '稍后' },
    ]);

    expect(message.content.map((part) => part.type)).toEqual(['reasoning', 'tool_call', 'text']);
    expect(extractMessageText(message)).toBe('稍后');
    expect(getToolCalls(message)).toHaveLength(1);
  });

  it('reasoning 支持提供方签名与加密思考数据', () => {
    const signed = createAssistantMessage([
      { type: 'reasoning', text: '思考', signature: 'sig-1' },
      { type: 'text', text: '回答' },
    ]);
    const redacted = createAssistantMessage([
      { type: 'reasoning', text: '', redactedData: 'opaque-data' },
    ]);

    expect(signed.content[0]).toEqual({ type: 'reasoning', text: '思考', signature: 'sig-1' });
    expect(redacted.content[0]).toEqual({
      type: 'reasoning',
      text: '',
      redactedData: 'opaque-data',
    });
    expect(Object.isFrozen(signed.content[0])).toBe(true);
    expect(Object.isFrozen(redacted.content[0])).toBe(true);
  });

  it('创建工具结果消息并缺省 isError=false', () => {
    const message = createToolMessage({ callId: 'c1', name: 'echo', content: 'ok' });

    expect(message.content).toEqual([
      { type: 'tool_result', callId: 'c1', name: 'echo', content: 'ok', isError: false },
    ]);
    expect(Object.isFrozen(message.content[0])).toBe(true);
  });

  it('独立克隆工具输入与结果元数据', () => {
    const input = { nested: { value: 1 } };
    const metadata = { source: 'test' };
    const call = createAssistantMessage([{ type: 'tool_call', callId: 'c1', name: 'echo', input }]);
    const result = createToolMessage({
      callId: 'c1',
      name: 'echo',
      content: 'ok',
      metadata,
    });

    input.nested.value = 2;
    metadata.source = 'changed';

    expect(getToolCalls(call)[0]?.input).toEqual({ nested: { value: 1 } });
    expect(result.content[0]).toMatchObject({ metadata: { source: 'test' } });
    expect(Object.isFrozen(getToolCalls(call)[0]?.input)).toBe(true);
  });
});

describe('工具名称', () => {
  it('接受 1 到 64 字符的合法名称', () => {
    expect(isValidToolName('a')).toBe(true);
    expect(isValidToolName('search_files-2')).toBe(true);
    expect(isValidToolName(`a${'b'.repeat(63)}`)).toBe(true);
  });

  it('拒绝非法名称', () => {
    expect(isValidToolName('')).toBe(false);
    expect(isValidToolName('1tool')).toBe(false);
    expect(isValidToolName('_tool')).toBe(false);
    expect(isValidToolName('tool.name')).toBe(false);
    expect(isValidToolName(`a${'b'.repeat(64)}`)).toBe(false);
    expect(isValidToolName(1)).toBe(false);
  });
});

describe('归一化校验', () => {
  it('接受普通对象输入并返回冻结副本', () => {
    const raw = { role: 'user', content: 'hi' };
    const normalized = normalizeAgentMessage(raw, '$.message');

    expect(normalized).toEqual({ role: 'user', content: [{ type: 'text', text: 'hi' }] });
    expect(Object.isFrozen(normalized)).toBe(true);
    expect(raw).toEqual({ role: 'user', content: 'hi' });
  });

  it.each([
    ['非对象', null],
    ['未知角色', { role: 'developer', content: 'x' }],
    ['空内容数组', { role: 'user', content: [] }],
    ['内容类型非法', { role: 'user', content: [{ type: 'unknown' }] }],
    [
      'system 带图片',
      { role: 'system', content: [{ type: 'image', mediaType: 'image/png', data: 'x' }] },
    ],
    [
      'user 带工具调用',
      { role: 'user', content: [{ type: 'tool_call', callId: 'c', name: 't', input: {} }] },
    ],
    [
      'assistant 带工具结果',
      {
        role: 'assistant',
        content: [{ type: 'tool_result', callId: 'c', name: 't', content: 'x', isError: false }],
      },
    ],
    ['tool 带文本', { role: 'tool', content: 'x' }],
    [
      'tool 带两个结果',
      {
        role: 'tool',
        content: [
          { type: 'tool_result', callId: 'c1', name: 't', content: 'x', isError: false },
          { type: 'tool_result', callId: 'c2', name: 't', content: 'y', isError: false },
        ],
      },
    ],
    ['text 非字符串', { role: 'user', content: [{ type: 'text', text: 1 }] }],
    ['reasoning 非字符串', { role: 'assistant', content: [{ type: 'reasoning', text: 1 }] }],
    [
      'reasoning 签名为空',
      { role: 'assistant', content: [{ type: 'reasoning', text: 'x', signature: ' ' }] },
    ],
    [
      'reasoning 加密数据为空',
      { role: 'assistant', content: [{ type: 'reasoning', text: 'x', redactedData: '' }] },
    ],
    [
      'reasoning 同时带签名与加密数据',
      {
        role: 'assistant',
        content: [{ type: 'reasoning', text: 'x', signature: 's', redactedData: 'd' }],
      },
    ],
    [
      '图片 MIME 非法',
      { role: 'user', content: [{ type: 'image', mediaType: 'text/plain', data: 'x' }] },
    ],
    [
      '图片数据为空',
      { role: 'user', content: [{ type: 'image', mediaType: 'image/png', data: ' ' }] },
    ],
    [
      '工具名非法',
      { role: 'assistant', content: [{ type: 'tool_call', callId: 'c', name: '1t', input: {} }] },
    ],
    [
      '调用 ID 为空',
      { role: 'assistant', content: [{ type: 'tool_call', callId: ' ', name: 't', input: {} }] },
    ],
    [
      '同一消息调用 ID 重复',
      {
        role: 'assistant',
        content: [
          { type: 'tool_call', callId: 'c1', name: 't', input: {} },
          { type: 'tool_call', callId: 'c1', name: 't', input: {} },
        ],
      },
    ],
    [
      '工具输入为函数',
      {
        role: 'assistant',
        content: [{ type: 'tool_call', callId: 'c', name: 't', input: () => 1 }],
      },
    ],
    [
      '工具输入缺省',
      { role: 'assistant', content: [{ type: 'tool_call', callId: 'c', name: 't' }] },
    ],
    [
      '工具结果内容非字符串',
      {
        role: 'tool',
        content: [{ type: 'tool_result', callId: 'c', name: 't', content: 1, isError: false }],
      },
    ],
    [
      '工具结果 isError 非布尔',
      {
        role: 'tool',
        content: [{ type: 'tool_result', callId: 'c', name: 't', content: 'x', isError: 'no' }],
      },
    ],
    [
      '工具结果 metadata 非 JSON',
      {
        role: 'tool',
        content: [
          {
            type: 'tool_result',
            callId: 'c',
            name: 't',
            content: 'x',
            isError: false,
            metadata: () => 1,
          },
        ],
      },
    ],
  ])('拒绝%s', (_name, value) => {
    expect(() => normalizeAgentMessage(value, '$.message')).toThrow(AgentMessageError);
  });

  it('错误携带稳定 code 与路径', () => {
    try {
      normalizeAgentMessage({ role: 'user', content: [] }, '$.messages[0]');
      expect.unreachable('应当抛错');
    } catch (error) {
      expect(error).toBeInstanceOf(AgentMessageError);
      expect((error as AgentMessageError).code).toBe('agent_message_error');
      expect((error as AgentMessageError).path).toBe('$.messages[0].content');
    }
  });

  it('危险键输入被包装为消息错误并保留 cause', () => {
    const input: unknown = JSON.parse('{"__proto__":{"polluted":true}}');
    try {
      createAssistantMessage([
        { type: 'tool_call', callId: 'c', name: 't', input: input as JsonValue },
      ]);
      expect.unreachable('应当抛错');
    } catch (error) {
      expect(error).toBeInstanceOf(AgentMessageError);
      expect((error as AgentMessageError).cause).toBeDefined();
    }
  });

  it('createMessage 在运行期拒绝非法角色', () => {
    expect(() => createMessage('developer' as never, 'x')).toThrow(AgentMessageError);
  });
});

describe('消息提取', () => {
  it('getToolCalls 只返回工具调用块且结果冻结', () => {
    const message = createAssistantMessage([
      { type: 'text', text: 'a' },
      { type: 'tool_call', callId: 'c1', name: 'x', input: {} },
      { type: 'tool_call', callId: 'c2', name: 'y', input: {} },
    ]);
    const calls = getToolCalls(message);

    expect(calls.map((call) => call.callId)).toEqual(['c1', 'c2']);
    expect(Object.isFrozen(calls)).toBe(true);
  });

  it('extractMessageText 只拼接 text 块', () => {
    const message = createAssistantMessage([
      { type: 'text', text: 'a' },
      { type: 'reasoning', text: 'hidden' },
      { type: 'text', text: 'b' },
    ]);

    expect(extractMessageText(message)).toBe('ab');
    expect(extractMessageText(createAssistantMessage([{ type: 'reasoning', text: 'x' }]))).toBe('');
  });

  it('extractMessageReasoning 只拼接 reasoning 块', () => {
    const message = createAssistantMessage([
      { type: 'reasoning', text: 'a', signature: 'sig' },
      { type: 'text', text: 'answer' },
      { type: 'reasoning', text: 'b' },
    ]);

    expect(extractMessageReasoning(message)).toBe('ab');
    expect(extractMessageReasoning(createAssistantMessage('x'))).toBe('');
  });
});
