/**
 * @file Agent 配置、会话集成与便捷入口测试。
 * @description 覆盖构造期/运行期参数校验、会话追加与一次性 API。
 * @module agent/agent-config.test
 */

import { describe, expect, it } from 'vitest';

import {
  AgentDefinitionError,
  AgentModelError,
  AgentStopReason,
  createAgent,
  createAgentSession,
  createTool,
  createToolRegistry,
  createUserMessage,
  runAgent,
  streamAgent,
  type AgentConfig,
  type ModelProvider,
  type ModelResponse,
} from './index.js';
import {
  collectEvents,
  createScriptedProvider,
  eventTypes,
  textResponse,
  toolCall,
  toolCallResponse,
} from './agent.test-utils.js';

const provider: ModelProvider = {
  id: 'p',
  async generate(): Promise<ModelResponse> {
    return textResponse('ok');
  },
};

function echoToolDefinition() {
  return createTool({
    name: 'echo',
    description: '回显',
    inputSchema: {},
    execute: () => 'ok',
  });
}

describe('配置校验', () => {
  it('配置必须为对象', () => {
    expect(() => createAgent(null as never)).toThrow(AgentDefinitionError);
  });

  it.each([
    ['provider 为 null', { provider: null }],
    ['provider 缺少 generate', { provider: { id: 'p' } }],
    ['provider.stream 非函数', { provider: { ...provider, stream: 1 } }],
    ['provider.id 为空', { provider: { ...provider, id: ' ' } }],
    ['model 为空', { model: '' }],
    ['system 为空', { system: ' ' }],
    ['maxSteps 为 0', { maxSteps: 0 }],
    ['maxSteps 为小数', { maxSteps: 1.5 }],
    ['maxSteps 为 Infinity', { maxSteps: Number.POSITIVE_INFINITY }],
    ['toolConcurrency 为 0', { toolConcurrency: 0 }],
    ['toolTimeoutMs 为 0', { toolTimeoutMs: 0 }],
    ['maxToolResultLength 为 0', { maxToolResultLength: 0 }],
    ['temperature 为 NaN', { temperature: Number.NaN }],
    ['maxOutputTokens 为 0', { maxOutputTokens: 0 }],
    ['thinking 为字符串', { thinking: 'on' }],
    ['thinking.isEnabled 非法', { thinking: { isEnabled: 1 } }],
    ['thinking.budgetTokens 为 0', { thinking: { budgetTokens: 0 } }],
    ['thinking.effort 非法', { thinking: { effort: 'extreme' } }],
    ['toolChoice 非法', { toolChoice: 'always' }],
    ['providerOptions 非 JSON', { providerOptions: () => 1 }],
    ['shouldStream 非布尔', { shouldStream: 'yes' }],
    ['maxModelRetries 为负', { maxModelRetries: -1 }],
    ['retryDelayMs 为负', { retryDelayMs: -1 }],
    ['shouldRetryModelError 非函数', { shouldRetryModelError: 1 }],
    ['approveToolCall 非函数', { approveToolCall: 1 }],
    ['工具重名', { tools: [echoToolDefinition(), echoToolDefinition()] }],
  ])('拒绝%s', (_name, patch) => {
    expect(() => createAgent({ provider, model: 'm', ...patch } as AgentConfig)).toThrow(
      AgentDefinitionError,
    );
  });

  it('运行选项必须为对象', async () => {
    const agent = createAgent({ provider, model: 'm' });

    await expect(agent.run([createUserMessage('go')], null as never)).rejects.toBeInstanceOf(
      AgentDefinitionError,
    );
  });

  it.each([
    ['maxSteps 为 0', { maxSteps: 0 }],
    ['shouldStream 非布尔', { shouldStream: 1 }],
    ['toolChoice 非法', { toolChoice: 'always' }],
    ['thinking.budgetTokens 为负', { thinking: { budgetTokens: -1 } }],
  ])('拒绝运行期覆盖 %s', async (_name, options) => {
    const agent = createAgent({ provider, model: 'm' });

    await expect(agent.run([createUserMessage('go')], options as never)).rejects.toBeInstanceOf(
      AgentDefinitionError,
    );
  });

  it('默认设置可正常构造', () => {
    const agent = createAgent({ provider, model: 'm' });

    expect(agent.providerId).toBe('p');
    expect(agent.model).toBe('m');
    expect(agent.tools.size).toBe(0);
    expect(Object.isFrozen(agent)).toBe(true);
  });

  it('工具注册表可通过 Agent 读取', () => {
    const registry = createToolRegistry([echoToolDefinition()]);
    const agent = createAgent({ provider, model: 'm', tools: [echoToolDefinition()] });

    expect(agent.tools.size).toBe(1);
    expect(agent.tools.descriptors).toEqual(registry.descriptors);
  });
});

describe('会话集成', () => {
  it('runInSession 只追加本次生成的消息', async () => {
    const { provider: scripted, requests } = createScriptedProvider([
      textResponse('一'),
      textResponse('二'),
    ]);
    const agent = createAgent({ provider: scripted, model: 'm', system: 'sys' });
    const session = createAgentSession({ messages: [createUserMessage('hi')] });

    const first = await agent.runInSession(session);
    expect(first.generatedMessages.map((message) => message.role)).toEqual(['assistant']);
    expect(session.messages.map((message) => message.role)).toEqual(['user', 'assistant']);

    await agent.runInSession(session);
    expect(session.messages.map((message) => message.role)).toEqual([
      'user',
      'assistant',
      'assistant',
    ]);
    expect(requests[1]?.messages.map((message) => message.role)).toEqual([
      'system',
      'user',
      'assistant',
    ]);
  });

  it('空会话在配置 system 时可以启动', async () => {
    const { provider: scripted, requests } = createScriptedProvider([textResponse('hello')]);
    const agent = createAgent({ provider: scripted, model: 'm', system: 'sys' });
    const session = createAgentSession();

    await agent.runInSession(session);

    expect(requests[0]?.messages.map((message) => message.role)).toEqual(['system']);
    expect(session.messages.map((message) => message.role)).toEqual(['assistant']);
  });

  it('运行失败时不修改会话', async () => {
    const failing: ModelProvider = {
      id: 'fail',
      async generate(): Promise<ModelResponse> {
        throw new Error('down');
      },
    };
    const agent = createAgent({ provider: failing, model: 'm' });
    const session = createAgentSession({ messages: [createUserMessage('hi')] });
    const before = session.messages;

    await expect(agent.runInSession(session)).rejects.toBeInstanceOf(AgentModelError);
    expect(session.messages).toBe(before);
  });
});

describe('便捷入口', () => {
  it('runAgent 返回结果', async () => {
    const { provider: scripted } = createScriptedProvider([textResponse('ok')]);

    const result = await runAgent({ provider: scripted, model: 'm' }, [createUserMessage('go')]);

    expect(result.stopReason).toBe(AgentStopReason.Completed);
    expect(result.finalMessage?.content).toEqual([{ type: 'text', text: 'ok' }]);
  });

  it('streamAgent 返回完整事件流', async () => {
    const { provider: scripted } = createScriptedProvider([textResponse('ok')]);

    const { events, result } = await collectEvents(
      streamAgent({ provider: scripted, model: 'm' }, [createUserMessage('go')]),
    );

    expect(eventTypes(events)).toEqual([
      'agent_start',
      'step_start',
      'assistant_message',
      'step_end',
      'agent_end',
    ]);
    expect(result.steps).toBe(1);
  });

  it('runInSession 可处理工具循环', async () => {
    const { provider: scripted } = createScriptedProvider([
      toolCallResponse([toolCall('c1', 'echo', {})]),
      textResponse('done'),
    ]);
    const agent = createAgent({
      provider: scripted,
      model: 'm',
      tools: [echoToolDefinition()],
    });
    const session = createAgentSession({ messages: [createUserMessage('go')] });

    await agent.runInSession(session);

    expect(session.messages.map((message) => message.role)).toEqual([
      'user',
      'assistant',
      'tool',
      'assistant',
    ]);
  });
});
