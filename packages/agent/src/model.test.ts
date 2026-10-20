/**
 * @file 模型契约校验测试。
 * @description 覆盖停止原因、工具选择、透传选项、响应与流事件归一化。
 * @module agent/model.test
 */

import { describe, expect, it } from 'vitest';

import { AgentDefinitionError, AgentModelError } from './errors.js';
import {
  EMPTY_TOKEN_USAGE,
  ModelStopReason,
  ThinkingEffort,
  createAssistantMessage,
  isModelStopReason,
  normalizeThinkingConfig,
  normalizeToolChoice,
} from './index.js';
import {
  normalizeModelResponse,
  normalizeModelStreamEvent,
  normalizeProviderOptions,
} from './model.js';

describe('isModelStopReason', () => {
  it('识别全部合法停止原因', () => {
    for (const reason of Object.values(ModelStopReason)) {
      expect(isModelStopReason(reason)).toBe(true);
    }
  });

  it('拒绝未知值', () => {
    expect(isModelStopReason('stop')).toBe(false);
    expect(isModelStopReason(1)).toBe(false);
  });
});

describe('normalizeToolChoice', () => {
  it.each(['auto', 'none', 'required'] as const)('接受 %s', (choice) => {
    expect(normalizeToolChoice(choice)).toBe(choice);
  });

  it('接受并冻结指定工具', () => {
    const choice = normalizeToolChoice({ type: 'tool', name: 'search' });

    expect(choice).toEqual({ type: 'tool', name: 'search' });
    expect(Object.isFrozen(choice)).toBe(true);
  });

  it.each([
    ['未知字符串', 'always'],
    ['null', null],
    ['错误 type', { type: 'function', name: 'search' }],
    ['非法名称', { type: 'tool', name: '1search' }],
    ['缺少名称', { type: 'tool' }],
  ])('拒绝%s', (_name, value) => {
    expect(() => normalizeToolChoice(value)).toThrow(AgentDefinitionError);
  });
});

describe('normalizeThinkingConfig', () => {
  it('布尔值映射为启用/关闭', () => {
    expect(normalizeThinkingConfig(true)).toEqual({ isEnabled: true });
    expect(normalizeThinkingConfig(false)).toEqual({ isEnabled: false });
  });

  it('接受空对象并缺省启用', () => {
    const config = normalizeThinkingConfig({});

    expect(config).toEqual({ isEnabled: true });
    expect(Object.isFrozen(config)).toBe(true);
  });

  it('接受预算与推理强度并冻结', () => {
    const config = normalizeThinkingConfig({
      isEnabled: true,
      budgetTokens: 2048,
      effort: ThinkingEffort.High,
    });

    expect(config).toEqual({ isEnabled: true, budgetTokens: 2048, effort: 'high' });
    expect(Object.isFrozen(config)).toBe(true);
  });

  it.each([
    ['null', null],
    ['字符串', 'on'],
    ['isEnabled 非布尔', { isEnabled: 1 }],
    ['isEnabled 为 null', { isEnabled: null }],
    ['预算为 0', { budgetTokens: 0 }],
    ['预算为负', { budgetTokens: -1 }],
    ['预算为小数', { budgetTokens: 1.5 }],
    ['预算溢出', { budgetTokens: Number.MAX_SAFE_INTEGER + 1 }],
    ['effort 非法', { effort: 'extreme' }],
  ])('拒绝%s', (_name, value) => {
    expect(() => normalizeThinkingConfig(value)).toThrow(AgentDefinitionError);
  });
});

describe('normalizeProviderOptions', () => {
  it('克隆并冻结 JSON 值', () => {
    const options = { nested: { value: 1 } };
    const normalized = normalizeProviderOptions(options);
    options.nested.value = 2;

    expect(normalized).toEqual({ nested: { value: 1 } });
    expect(Object.isFrozen(normalized)).toBe(true);
  });

  it('拒绝非 JSON 值', () => {
    expect(() => normalizeProviderOptions(() => 1)).toThrow(AgentDefinitionError);
  });

  it('拒绝危险键并保留 cause', () => {
    const value = JSON.parse('{"__proto__":{"polluted":true}}') as unknown;
    try {
      normalizeProviderOptions(value);
      expect.unreachable('应当抛错');
    } catch (error) {
      expect(error).toBeInstanceOf(AgentDefinitionError);
      expect((error as AgentDefinitionError).cause).toBeDefined();
    }
  });
});

describe('normalizeModelResponse', () => {
  it('接受 assistant 消息并补全零用量', () => {
    const response = normalizeModelResponse('test', {
      message: createAssistantMessage('ok'),
      stopReason: 'end',
    });

    expect(response.message).toEqual(createAssistantMessage('ok'));
    expect(response.stopReason).toBe('end');
    expect(response.usage).toBe(EMPTY_TOKEN_USAGE);
    expect(Object.isFrozen(response)).toBe(true);
  });

  it('接受并校验 usage', () => {
    const response = normalizeModelResponse('test', {
      message: createAssistantMessage('ok'),
      stopReason: 'max_tokens',
      usage: { inputTokens: 1, outputTokens: 2, totalTokens: 3 },
    });

    expect(response.usage).toEqual({ inputTokens: 1, outputTokens: 2, totalTokens: 3 });
  });

  it.each([
    ['非对象', null],
    ['消息非法', { message: { role: 'user', content: 'x' }, stopReason: 'end' }],
    ['消息角色错误', { message: createUserMessageForTest(), stopReason: 'end' }],
    ['停止原因非法', { message: createAssistantMessage('x'), stopReason: 'stop' }],
    [
      'usage 非法',
      {
        message: createAssistantMessage('x'),
        stopReason: 'end',
        usage: { inputTokens: -1, outputTokens: 1 },
      },
    ],
  ])('拒绝%s', (_name, value) => {
    expect(() => normalizeModelResponse('test', value)).toThrow(AgentModelError);
  });

  it('错误包含 providerId', () => {
    try {
      normalizeModelResponse('deepseek', null);
      expect.unreachable('应当抛错');
    } catch (error) {
      expect(error).toBeInstanceOf(AgentModelError);
      expect((error as AgentModelError).providerId).toBe('deepseek');
    }
  });
});

describe('normalizeModelStreamEvent', () => {
  it('接受文本与推理增量', () => {
    expect(normalizeModelStreamEvent('test', { type: 'text_delta', text: 'a' })).toEqual({
      type: 'text_delta',
      text: 'a',
    });
    expect(normalizeModelStreamEvent('test', { type: 'reasoning_delta', text: 'b' })).toEqual({
      type: 'reasoning_delta',
      text: 'b',
    });
  });

  it('接受工具调用增量及可选字段', () => {
    expect(
      normalizeModelStreamEvent('test', {
        type: 'tool_call_delta',
        index: 0,
        callId: 'c1',
        name: 'search',
        argumentsDelta: '{"q":',
      }),
    ).toEqual({
      type: 'tool_call_delta',
      index: 0,
      callId: 'c1',
      name: 'search',
      argumentsDelta: '{"q":',
    });
  });

  it('接受最终 response 事件', () => {
    const event = normalizeModelStreamEvent('test', {
      type: 'response',
      response: { message: createAssistantMessage('ok'), stopReason: 'end' },
    });

    expect(event.type).toBe('response');
    if (event.type === 'response') {
      expect(event.response.usage).toEqual(EMPTY_TOKEN_USAGE);
    }
  });

  it.each([
    ['非对象', null],
    ['未知类型', { type: 'unknown' }],
    ['文本非字符串', { type: 'text_delta', text: 1 }],
    ['推理非字符串', { type: 'reasoning_delta', text: 1 }],
    ['索引为负', { type: 'tool_call_delta', index: -1, argumentsDelta: '' }],
    ['索引为小数', { type: 'tool_call_delta', index: 1.5, argumentsDelta: '' }],
    ['参数片段非字符串', { type: 'tool_call_delta', index: 0, argumentsDelta: 1 }],
    ['callId 为空', { type: 'tool_call_delta', index: 0, callId: ' ', argumentsDelta: '' }],
    ['工具名非法', { type: 'tool_call_delta', index: 0, name: '1x', argumentsDelta: '' }],
    ['response 非法', { type: 'response', response: null }],
  ])('拒绝%s', (_name, value) => {
    expect(() => normalizeModelStreamEvent('test', value)).toThrow(AgentModelError);
  });
});

/** 构造 user 消息用于非法角色测试。 */
function createUserMessageForTest() {
  return { role: 'user', content: [{ type: 'text', text: 'x' }] };
}
