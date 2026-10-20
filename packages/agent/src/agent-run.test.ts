/**
 * @file Agent 运行循环测试。
 * @description 覆盖单轮、工具循环、错误分类、轮数上限与取消语义。
 * @module agent/agent-run.test
 */

import { describe, expect, it } from 'vitest';

import {
  AgentMessageError,
  AgentModelError,
  AgentStopReason,
  AgentToolError,
  ModelStopReason,
  createAgent,
  createTool,
  createUserMessage,
  createAssistantMessage,
  extractMessageText,
  getToolCalls,
  type AgentMessage,
  type ModelRequest,
  type ModelResponse,
} from './index.js';
import {
  collectEvents,
  createScriptedProvider,
  deferred,
  eventTypes,
  textResponse,
  toolCall,
  toolCallResponse,
  usageOf,
} from './agent.test-utils.js';

describe('基础运行', () => {
  it('构造模型请求、返回完整结果并冻结', async () => {
    const { provider, requests } = createScriptedProvider([
      textResponse('你好', ModelStopReason.End, usageOf(10, 5)),
    ]);
    const agent = createAgent({
      provider,
      model: 'deepseek-chat',
      system: '你是助手',
      temperature: 0.3,
      maxOutputTokens: 256,
      providerOptions: { thinking: false },
      thinking: { budgetTokens: 2_048, effort: 'high' },
      toolChoice: 'auto',
    });

    const result = await agent.run([createUserMessage('hi')]);

    expect(requests).toHaveLength(1);
    expect(requests[0]).toMatchObject({
      model: 'deepseek-chat',
      temperature: 0.3,
      maxOutputTokens: 256,
      providerOptions: { thinking: false },
      thinking: { isEnabled: true, budgetTokens: 2_048, effort: 'high' },
      toolChoice: 'auto',
      tools: [],
    });
    expect(requests[0]?.messages.map((message) => message.role)).toEqual(['system', 'user']);
    expect(Object.isFrozen(requests[0]?.messages)).toBe(true);
    expect(result.messages.map((message) => message.role)).toEqual(['system', 'user', 'assistant']);
    expect(result.generatedMessages).toHaveLength(1);
    expect(result.finalMessage).toBe(result.generatedMessages[0]);
    expect(extractMessageText(result.finalMessage as AgentMessage)).toBe('你好');
    expect(result.steps).toBe(1);
    expect(result.usage).toEqual({ inputTokens: 10, outputTokens: 5, totalTokens: 15 });
    expect(result.stopReason).toBe(AgentStopReason.Completed);
    expect(result.modelStopReason).toBe(ModelStopReason.End);
    expect(result.retryCount).toBe(0);
    expect(Object.isFrozen(result)).toBe(true);
    expect(Object.isFrozen(result.messages)).toBe(true);
    expect(Object.isFrozen(result.generatedMessages)).toBe(true);
  });

  it('单次运行可覆盖思考配置且请求快照冻结', async () => {
    const { provider, requests } = createScriptedProvider([textResponse('ok')]);
    const agent = createAgent({
      provider,
      model: 'm',
      thinking: { budgetTokens: 1_024 },
    });

    await agent.run([createUserMessage('hi')], {
      thinking: { isEnabled: false, effort: 'low' },
    });

    expect(requests[0]?.thinking).toEqual({ isEnabled: false, effort: 'low' });
    expect(Object.isFrozen(requests[0]?.thinking)).toBe(true);
  });

  it('未配置 thinking 时不向请求添加字段', async () => {
    const { provider, requests } = createScriptedProvider([textResponse('ok')]);
    const agent = createAgent({ provider, model: 'm' });

    await agent.run([createUserMessage('hi')]);

    expect(requests[0]).not.toHaveProperty('thinking');
  });

  it('保留 assistant 推理签名并可通过结果提取', async () => {
    const message = createAssistantMessage([
      { type: 'reasoning', text: '先思考', signature: 'sig-1' },
      { type: 'text', text: '答案' },
    ]);
    const { provider } = createScriptedProvider([{ message, stopReason: ModelStopReason.End }]);
    const agent = createAgent({ provider, model: 'm' });

    const { events, result } = await collectEvents(agent.stream([createUserMessage('hi')]));
    const assistantEvent = events.find((event) => event.type === 'assistant_message');

    expect(assistantEvent?.type).toBe('assistant_message');
    if (assistantEvent?.type === 'assistant_message') {
      expect(assistantEvent.message.content[0]).toEqual({
        type: 'reasoning',
        text: '先思考',
        signature: 'sig-1',
      });
    }
    expect(result.finalMessage?.content[0]).toEqual({
      type: 'reasoning',
      text: '先思考',
      signature: 'sig-1',
    });
  });

  it('事件流顺序固定且最终结果一致', async () => {
    const { provider } = createScriptedProvider([textResponse('ok')]);
    const agent = createAgent({ provider, model: 'm' });

    const { events, result } = await collectEvents(agent.stream([createUserMessage('hi')]));

    expect(eventTypes(events)).toEqual([
      'agent_start',
      'step_start',
      'assistant_message',
      'step_end',
      'agent_end',
    ]);
    expect(Object.isFrozen(events[0])).toBe(true);
    expect(events.at(-1)).toEqual({ type: 'agent_end', result });
  });

  it('重复使用完整结果不会重复注入 system', async () => {
    const { provider, requests } = createScriptedProvider([textResponse('一'), textResponse('二')]);
    const agent = createAgent({ provider, model: 'm', system: 'sys' });

    const first = await agent.run([createUserMessage('a')]);
    const second = await agent.run([...first.messages, createUserMessage('b')]);

    expect(second.messages.map((message) => message.role)).toEqual([
      'system',
      'user',
      'assistant',
      'user',
      'assistant',
    ]);
    expect(requests[1]?.messages.filter((message) => message.role === 'system')).toHaveLength(1);
  });

  it('输入首条 system 会覆盖配置 system', async () => {
    const { provider, requests } = createScriptedProvider([textResponse('ok')]);
    const agent = createAgent({ provider, model: 'm', system: 'configured' });

    await agent.run([
      { role: 'system', content: [{ type: 'text', text: 'custom' }] },
      createUserMessage('hi'),
    ]);

    expect(requests[0]?.messages[0]).toEqual({
      role: 'system',
      content: [{ type: 'text', text: 'custom' }],
    });
  });

  it('没有消息且没有 system 时拒绝运行', async () => {
    const { provider } = createScriptedProvider([]);
    const agent = createAgent({ provider, model: 'm' });

    await expect(agent.run([])).rejects.toThrow('messages 不能为空');
  });
});

describe('工具循环', () => {
  it('执行工具、追加结果并在下一轮得到最终回答', async () => {
    const { provider, requests } = createScriptedProvider([
      toolCallResponse([toolCall('c1', 'echo', { text: 'hi' })], usageOf(4, 2)),
      textResponse('完成', ModelStopReason.End, usageOf(3, 1)),
    ]);
    const tool = createTool({
      name: 'echo',
      description: '回显',
      inputSchema: { type: 'object' },
      parseInput: (input) => ({ text: String((input as { text?: unknown }).text) }),
      execute: (input, context) => ({
        content: `${input.text}:${context.callId}:${context.signal.aborted}`,
        metadata: { source: 'test' },
      }),
    });
    const agent = createAgent({ provider, model: 'm', tools: [tool] });

    const result = await agent.run([createUserMessage('go')]);

    expect(requests).toHaveLength(2);
    expect(requests[1]?.messages.map((message) => message.role)).toEqual([
      'user',
      'assistant',
      'tool',
    ]);
    expect(requests[1]?.messages[2]?.content[0]).toMatchObject({
      type: 'tool_result',
      callId: 'c1',
      name: 'echo',
      content: 'hi:c1:false',
      isError: false,
      metadata: { source: 'test' },
    });
    expect(result.generatedMessages.map((message) => message.role)).toEqual([
      'assistant',
      'tool',
      'assistant',
    ]);
    expect(result.steps).toBe(2);
    expect(result.usage).toEqual({ inputTokens: 7, outputTokens: 3, totalTokens: 10 });
    expect(result.stopReason).toBe(AgentStopReason.Completed);
    expect(extractMessageText(result.finalMessage as AgentMessage)).toBe('完成');
  });

  it('工具事件按开始/结束成对出现', async () => {
    const { provider } = createScriptedProvider([
      toolCallResponse([toolCall('c1', 'echo', {})]),
      textResponse('ok'),
    ]);
    const tool = createTool({
      name: 'echo',
      description: '回显',
      inputSchema: {},
      execute: () => 'ok',
    });
    const agent = createAgent({ provider, model: 'm', tools: [tool] });

    const { events } = await collectEvents(agent.stream([createUserMessage('go')]));
    const toolEvents = events.filter(
      (event) => event.type === 'tool_start' || event.type === 'tool_end',
    );

    expect(toolEvents.map((event) => event.type)).toEqual(['tool_start', 'tool_end']);
    expect(Object.isFrozen(toolEvents[0])).toBe(true);
    const assistantEvent = events.find((event) => event.type === 'assistant_message');
    expect(assistantEvent?.type).toBe('assistant_message');
    if (assistantEvent?.type === 'assistant_message') {
      expect(getToolCalls(assistantEvent.message)).toHaveLength(1);
    }
  });

  it('未知工具产生错误结果并继续对话', async () => {
    const { provider, requests } = createScriptedProvider([
      toolCallResponse([toolCall('c1', 'missing', {})]),
      textResponse('已恢复'),
    ]);
    const agent = createAgent({ provider, model: 'm' });

    const result = await agent.run([createUserMessage('go')]);
    const toolMessage = requests[1]?.messages.at(-1);

    expect(toolMessage?.content[0]).toMatchObject({
      type: 'tool_result',
      content: '未找到工具 "missing"',
      isError: true,
    });
    expect(result.stopReason).toBe(AgentStopReason.Completed);
  });

  it('工具抛错被转换为错误结果', async () => {
    const { provider, requests } = createScriptedProvider([
      toolCallResponse([toolCall('c1', 'boom', {})]),
      textResponse('ok'),
    ]);
    const tool = createTool({
      name: 'boom',
      description: '抛错',
      inputSchema: {},
      execute: () => {
        throw new Error('底层失败');
      },
    });
    const agent = createAgent({ provider, model: 'm', tools: [tool] });

    await agent.run([createUserMessage('go')]);

    expect(requests[1]?.messages.at(-1)?.content[0]).toMatchObject({
      content: '工具执行失败: 底层失败',
      isError: true,
    });
  });

  it('输入解析失败被转换为错误结果', async () => {
    const { provider, requests } = createScriptedProvider([
      toolCallResponse([toolCall('c1', 'parse', {})]),
      textResponse('ok'),
    ]);
    const tool = createTool({
      name: 'parse',
      description: '解析',
      inputSchema: {},
      parseInput: () => {
        throw new Error('参数缺失');
      },
      execute: () => 'never',
    });
    const agent = createAgent({ provider, model: 'm', tools: [tool] });

    await agent.run([createUserMessage('go')]);

    expect(requests[1]?.messages.at(-1)?.content[0]).toMatchObject({
      content: '工具参数无效: 参数缺失',
      isError: true,
    });
  });

  it('工具返回非法结果时抛 AgentToolError', async () => {
    const { provider } = createScriptedProvider([toolCallResponse([toolCall('c1', 'bad', {})])]);
    const tool = createTool({
      name: 'bad',
      description: '非法',
      inputSchema: {},
      execute: () => 42 as never,
    });
    const agent = createAgent({ provider, model: 'm', tools: [tool] });

    await expect(agent.run([createUserMessage('go')])).rejects.toBeInstanceOf(AgentToolError);
  });

  it('重复 callId 在消息构造期被拒绝', () => {
    expect(() =>
      toolCallResponse([toolCall('c1', 'echo', {}), toolCall('c1', 'echo', {})]),
    ).toThrow(AgentMessageError);
  });
});

describe('模型错误', () => {
  it('提供方抛错被包装并保留 cause', async () => {
    const cause = new Error('network');
    const provider = {
      id: 'broken',
      generate: async (): Promise<ModelResponse> => {
        throw cause;
      },
    };
    const agent = createAgent({ provider, model: 'm' });

    try {
      await agent.run([createUserMessage('go')]);
      expect.unreachable('应当抛错');
    } catch (error) {
      expect(error).toBeInstanceOf(AgentModelError);
      expect((error as AgentModelError).providerId).toBe('broken');
      expect((error as AgentModelError).cause).toBe(cause);
    }
  });

  it.each([
    ['消息角色错误', { message: createUserMessage('x'), stopReason: 'end' }],
    ['停止原因非法', { message: createAssistantMessage('x'), stopReason: 'stop' }],
    [
      'usage 非法',
      {
        message: createAssistantMessage('x'),
        stopReason: 'end',
        usage: { inputTokens: -1, outputTokens: 0, totalTokens: -1 },
      },
    ],
  ])('拒绝%s', async (_name, response) => {
    const { provider } = createScriptedProvider([response as ModelResponse]);
    const agent = createAgent({ provider, model: 'm' });

    await expect(agent.run([createUserMessage('go')])).rejects.toBeInstanceOf(AgentModelError);
  });

  it('stopReason=tool_calls 但没有调用时抛协议错误', async () => {
    const { provider } = createScriptedProvider([
      textResponse('只有文本', ModelStopReason.ToolCalls),
    ]);
    const agent = createAgent({ provider, model: 'm' });

    await expect(agent.run([createUserMessage('go')])).rejects.toThrow('stopReason=tool_calls');
  });
});

describe('轮数上限', () => {
  it('最后一轮执行工具后以 max_steps 结束', async () => {
    const { provider, requests } = createScriptedProvider([
      toolCallResponse([toolCall('c1', 'echo', {})]),
    ]);
    const tool = createTool({
      name: 'echo',
      description: '回显',
      inputSchema: {},
      execute: () => 'ok',
    });
    const agent = createAgent({ provider, model: 'm', tools: [tool], maxSteps: 1 });

    const result = await agent.run([createUserMessage('go')]);

    expect(requests).toHaveLength(1);
    expect(result.steps).toBe(1);
    expect(result.stopReason).toBe(AgentStopReason.MaxSteps);
    expect(result.finalMessage).toBeUndefined();
    expect(result.generatedMessages.map((message) => message.role)).toEqual(['assistant', 'tool']);
  });
});

describe('取消', () => {
  it('启动前已取消时不调用模型', async () => {
    const { provider, requests } = createScriptedProvider([textResponse('never')]);
    const agent = createAgent({ provider, model: 'm' });
    const controller = new AbortController();
    controller.abort();

    const { events, result } = await collectEvents(
      agent.stream([createUserMessage('go')], { signal: controller.signal }),
    );

    expect(requests).toHaveLength(0);
    expect(result.stopReason).toBe(AgentStopReason.Aborted);
    expect(result.steps).toBe(0);
    expect(result.generatedMessages).toEqual([]);
    expect(eventTypes(events)).toEqual(['agent_start', 'agent_end']);
  });

  it('模型调用期间取消不抛错', async () => {
    const started = deferred<void>();
    const provider = {
      id: 'slow',
      async generate(
        _request: ModelRequest,
        options?: { signal?: AbortSignal },
      ): Promise<ModelResponse> {
        started.resolve();
        await new Promise<void>((_resolve, reject) => {
          options?.signal?.addEventListener('abort', () => reject(new Error('aborted')), {
            once: true,
          });
        });
        return textResponse('late');
      },
    };
    const agent = createAgent({ provider, model: 'm' });
    const controller = new AbortController();
    const promise = agent.run([createUserMessage('go')], { signal: controller.signal });

    await started.promise;
    controller.abort();
    const result = await promise;

    expect(result.stopReason).toBe(AgentStopReason.Aborted);
    expect(result.steps).toBe(0);
    expect(result.generatedMessages).toEqual([]);
  });

  it('工具执行期间取消会追加取消结果并结束', async () => {
    const started = deferred<void>();
    let toolSignalAborted = false;
    const { provider } = createScriptedProvider([toolCallResponse([toolCall('c1', 'slow', {})])]);
    const tool = createTool({
      name: 'slow',
      description: '等待取消',
      inputSchema: {},
      async execute(_input, context): Promise<string> {
        started.resolve();
        await new Promise<void>((resolve) => {
          context.signal.addEventListener(
            'abort',
            () => {
              toolSignalAborted = context.signal.aborted;
              resolve();
            },
            { once: true },
          );
        });
        return 'late';
      },
    });
    const agent = createAgent({ provider, model: 'm', tools: [tool] });
    const controller = new AbortController();
    const promise = agent.run([createUserMessage('go')], { signal: controller.signal });

    await started.promise;
    controller.abort();
    const result = await promise;

    expect(toolSignalAborted).toBe(true);
    expect(result.stopReason).toBe(AgentStopReason.Aborted);
    expect(result.steps).toBe(1);
    expect(result.generatedMessages.map((message) => message.role)).toEqual(['assistant', 'tool']);
    expect(result.generatedMessages[1]?.content[0]).toMatchObject({
      type: 'tool_result',
      content: '工具调用已取消',
      isError: true,
    });
  });
});
