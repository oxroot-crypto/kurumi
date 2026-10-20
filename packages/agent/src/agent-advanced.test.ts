/**
 * @file Agent 高级能力测试。
 * @description 覆盖流式、重试、审批、并发、超时、截断与运行期覆盖。
 * @module agent/agent-advanced.test
 */

import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  AgentDefinitionError,
  AgentModelError,
  AgentStopReason,
  AgentToolError,
  ModelStopReason,
  createAgent,
  createTool,
  createUserMessage,
  type ModelCallOptions,
  type ModelProvider,
  type ModelRequest,
  type ModelResponse,
  type ModelStreamEvent,
} from './index.js';
import {
  collectEvents,
  createScriptedProvider,
  createStreamingProvider,
  deferred,
  textResponse,
  toolCall,
  toolCallResponse,
  usageOf,
} from './agent.test-utils.js';

afterEach(() => {
  vi.useRealTimers();
});

function echoTool(overrides: { timeoutMs?: number } = {}) {
  return createTool({
    name: 'echo',
    description: '回显',
    inputSchema: { type: 'object' },
    ...(overrides.timeoutMs === undefined ? {} : { timeoutMs: overrides.timeoutMs }),
    execute: (input) => JSON.stringify(input),
  });
}

describe('流式', () => {
  it('转发增量并用最终 response 组装消息', async () => {
    let streamCount = 0;
    const provider: ModelProvider = {
      id: 'stream',
      async generate(): Promise<ModelResponse> {
        throw new Error('不应调用 generate');
      },
      async *stream(): AsyncGenerator<ModelStreamEvent, void, void> {
        streamCount += 1;
        if (streamCount === 1) {
          yield { type: 'text_delta', text: '准备' };
          yield { type: 'reasoning_delta', text: '思考' };
          yield {
            type: 'tool_call_delta',
            index: 0,
            callId: 'c1',
            name: 'echo',
            argumentsDelta: '{}',
          };
          yield {
            type: 'response',
            response: toolCallResponse([toolCall('c1', 'echo', {})], usageOf(2, 1)),
          };
          return;
        }
        yield { type: 'text_delta', text: '完成' };
        yield {
          type: 'response',
          response: textResponse('完成', ModelStopReason.End, usageOf(1, 1)),
        };
      },
    };
    const agent = createAgent({ provider, model: 'm', tools: [echoTool()] });

    const { events, result } = await collectEvents(agent.stream([createUserMessage('go')]));

    const deltas = events.filter((event) => event.type === 'model_delta');
    expect(deltas).toHaveLength(4);
    expect(deltas.map((event) => (event.type === 'model_delta' ? event.delta.type : ''))).toEqual([
      'text_delta',
      'reasoning_delta',
      'tool_call_delta',
      'text_delta',
    ]);
    expect(streamCount).toBe(2);
    expect(result.steps).toBe(2);
    expect(result.usage).toEqual({ inputTokens: 3, outputTokens: 2, totalTokens: 5 });
    expect(result.finalMessage?.content).toEqual([{ type: 'text', text: '完成' }]);
  });

  it('shouldStream=false 时使用 generate', async () => {
    const { provider, counts } = createStreamingProvider(
      [{ type: 'response', response: textResponse('stream') }],
      textResponse('generate'),
    );
    const agent = createAgent({ provider, model: 'm', shouldStream: false });

    const result = await agent.run([createUserMessage('go')]);

    expect(counts.generate).toBe(1);
    expect(counts.stream).toBe(0);
    expect(result.finalMessage?.content).toEqual([{ type: 'text', text: 'generate' }]);
  });

  it('流缺少最终 response 时抛协议错误', async () => {
    const provider: ModelProvider = {
      id: 'broken',
      async generate(): Promise<ModelResponse> {
        throw new Error('不应调用 generate');
      },
      async *stream(): AsyncGenerator<ModelStreamEvent, void, void> {
        yield { type: 'text_delta', text: '半截' };
      },
    };
    const agent = createAgent({ provider, model: 'm' });

    await expect(agent.run([createUserMessage('go')])).rejects.toBeInstanceOf(AgentModelError);
  });

  it('流事件非法时抛模型错误', async () => {
    const provider: ModelProvider = {
      id: 'broken',
      async generate(): Promise<ModelResponse> {
        throw new Error('不应调用 generate');
      },
      async *stream(): AsyncGenerator<ModelStreamEvent, void, void> {
        yield { type: 'text_delta', text: 1 } as never;
      },
    };
    const agent = createAgent({ provider, model: 'm' });

    await expect(agent.run([createUserMessage('go')])).rejects.toBeInstanceOf(AgentModelError);
  });

  it('流式期间取消返回 aborted', async () => {
    const started = deferred<void>();
    const provider: ModelProvider = {
      id: 'slow',
      async generate(): Promise<ModelResponse> {
        throw new Error('不应调用 generate');
      },
      async *stream(
        _request: ModelRequest,
        options?: ModelCallOptions,
      ): AsyncGenerator<ModelStreamEvent, void, void> {
        yield { type: 'text_delta', text: 'a' };
        started.resolve();
        await new Promise<void>((resolve) => {
          options?.signal?.addEventListener('abort', () => resolve(), { once: true });
        });
        yield { type: 'text_delta', text: 'b' };
      },
    };
    const agent = createAgent({ provider, model: 'm' });
    const controller = new AbortController();
    const promise = agent.run([createUserMessage('go')], { signal: controller.signal });

    await started.promise;
    controller.abort();
    const result = await promise;

    expect(result.stopReason).toBe(AgentStopReason.Aborted);
    expect(result.generatedMessages).toEqual([]);
  });
});

describe('重试', () => {
  it('失败后重试并在事件中暴露序列化错误', async () => {
    let count = 0;
    const provider: ModelProvider = {
      id: 'flaky',
      async generate(): Promise<ModelResponse> {
        count += 1;
        if (count < 3) throw new Error(`失败 ${String(count)}`);
        return textResponse('ok');
      },
    };
    const agent = createAgent({
      provider,
      model: 'm',
      maxModelRetries: 2,
      retryDelayMs: 0,
    });

    const { events, result } = await collectEvents(agent.stream([createUserMessage('go')]));
    const retries = events.filter((event) => event.type === 'model_retry');

    expect(count).toBe(3);
    expect(retries).toHaveLength(2);
    expect(retries[0]).toMatchObject({ type: 'model_retry', step: 1, attempt: 1, delayMs: 0 });
    expect(result.retryCount).toBe(2);
    expect(result.finalMessage?.content).toEqual([{ type: 'text', text: 'ok' }]);
  });

  it('shouldRetryModelError=false 时立即失败', async () => {
    let count = 0;
    const provider: ModelProvider = {
      id: 'broken',
      async generate(): Promise<ModelResponse> {
        count += 1;
        throw new Error('down');
      },
    };
    const agent = createAgent({
      provider,
      model: 'm',
      maxModelRetries: 3,
      shouldRetryModelError: () => false,
    });

    await expect(agent.run([createUserMessage('go')])).rejects.toBeInstanceOf(AgentModelError);
    expect(count).toBe(1);
  });

  it('重试判定回调抛错时包装为模型错误', async () => {
    const provider: ModelProvider = {
      id: 'broken',
      async generate(): Promise<ModelResponse> {
        throw new Error('down');
      },
    };
    const agent = createAgent({
      provider,
      model: 'm',
      maxModelRetries: 1,
      shouldRetryModelError: () => {
        throw new Error('bad predicate');
      },
    });

    await expect(agent.run([createUserMessage('go')])).rejects.toThrow('重试判定回调失败');
  });

  it('按 retryDelayMs 等待后重试', async () => {
    vi.useFakeTimers();
    let count = 0;
    const provider: ModelProvider = {
      id: 'flaky',
      async generate(): Promise<ModelResponse> {
        count += 1;
        if (count === 1) throw new Error('first');
        return textResponse('ok');
      },
    };
    const agent = createAgent({
      provider,
      model: 'm',
      maxModelRetries: 1,
      retryDelayMs: 100,
    });

    const promise = agent.run([createUserMessage('go')]);
    await vi.advanceTimersByTimeAsync(100);
    const result = await promise;

    expect(count).toBe(2);
    expect(result.retryCount).toBe(1);
  });

  it('重试等待期间取消返回 aborted', async () => {
    vi.useFakeTimers();
    const started = deferred<void>();
    const provider: ModelProvider = {
      id: 'broken',
      async generate(): Promise<ModelResponse> {
        started.resolve();
        throw new Error('down');
      },
    };
    const agent = createAgent({
      provider,
      model: 'm',
      maxModelRetries: 2,
      retryDelayMs: 1_000,
    });
    const controller = new AbortController();
    const promise = agent.run([createUserMessage('go')], { signal: controller.signal });

    await started.promise;
    await Promise.resolve();
    await Promise.resolve();
    controller.abort();
    const result = await promise;

    expect(result.stopReason).toBe(AgentStopReason.Aborted);
    expect(result.retryCount).toBe(1);
  });
});

describe('工具审批', () => {
  it('拒绝时不执行工具并回传错误结果', async () => {
    let executed = 0;
    const { provider, requests } = createScriptedProvider([
      toolCallResponse([toolCall('c1', 'echo', {})]),
      textResponse('ok'),
    ]);
    const tool = createTool({
      name: 'echo',
      description: '回显',
      inputSchema: {},
      execute: () => {
        executed += 1;
        return 'ok';
      },
    });
    const agent = createAgent({
      provider,
      model: 'm',
      tools: [tool],
      approveToolCall: () => false,
    });

    await agent.run([createUserMessage('go')]);

    expect(executed).toBe(0);
    expect(requests[1]?.messages.at(-1)?.content[0]).toMatchObject({
      content: '工具调用未获批准',
      isError: true,
    });
  });

  it('异步批准后执行工具', async () => {
    const { provider } = createScriptedProvider([
      toolCallResponse([toolCall('c1', 'echo', {})]),
      textResponse('ok'),
    ]);
    const approvals: string[] = [];
    const tool = echoTool();
    const agent = createAgent({
      provider,
      model: 'm',
      tools: [tool],
      approveToolCall: async (call, context) => {
        approvals.push(`${call.name}:${String(context.messages.length)}`);
        return true;
      },
    });

    await agent.run([createUserMessage('go')]);

    expect(approvals).toEqual(['echo:2']);
  });

  it('审批回调抛错时抛 AgentToolError', async () => {
    const { provider } = createScriptedProvider([toolCallResponse([toolCall('c1', 'echo', {})])]);
    const agent = createAgent({
      provider,
      model: 'm',
      tools: [echoTool()],
      approveToolCall: () => {
        throw new Error('denied');
      },
    });

    await expect(agent.run([createUserMessage('go')])).rejects.toBeInstanceOf(AgentToolError);
  });

  it('审批回调返回非布尔值时抛 AgentToolError', async () => {
    const { provider } = createScriptedProvider([toolCallResponse([toolCall('c1', 'echo', {})])]);
    const agent = createAgent({
      provider,
      model: 'm',
      tools: [echoTool()],
      approveToolCall: (() => 'yes') as never,
    });

    await expect(agent.run([createUserMessage('go')])).rejects.toBeInstanceOf(AgentToolError);
  });

  it('审批期间取消返回取消结果', async () => {
    const started = deferred<void>();
    const { provider } = createScriptedProvider([toolCallResponse([toolCall('c1', 'echo', {})])]);
    const agent = createAgent({
      provider,
      model: 'm',
      tools: [echoTool()],
      approveToolCall: async () => {
        started.resolve();
        await new Promise<void>(() => {
          // 故意保持 pending，等待 Agent 取消。
        });
        return true;
      },
    });
    const controller = new AbortController();
    const promise = agent.run([createUserMessage('go')], { signal: controller.signal });

    await started.promise;
    controller.abort();
    const result = await promise;

    expect(result.stopReason).toBe(AgentStopReason.Aborted);
    expect(result.generatedMessages.at(-1)?.content[0]).toMatchObject({
      content: '工具调用已取消',
      isError: true,
    });
  });
});

describe('工具并发', () => {
  it('遵守并发上限并保持结果顺序', async () => {
    let active = 0;
    let maxActive = 0;
    const { provider, requests } = createScriptedProvider([
      toolCallResponse([
        toolCall('c1', 'echo', { id: 1 }),
        toolCall('c2', 'echo', { id: 2 }),
        toolCall('c3', 'echo', { id: 3 }),
      ]),
      textResponse('done'),
    ]);
    const tool = createTool({
      name: 'echo',
      description: '并发回显',
      inputSchema: {},
      execute: async (input) => {
        active += 1;
        maxActive = Math.max(maxActive, active);
        await Promise.resolve();
        active -= 1;
        return JSON.stringify(input);
      },
    });
    const agent = createAgent({
      provider,
      model: 'm',
      tools: [tool],
      toolConcurrency: 2,
    });

    await agent.run([createUserMessage('go')]);

    expect(maxActive).toBe(2);
    expect(
      requests[1]?.messages
        .filter((message) => message.role === 'tool')
        .map((message) => (message.content[0] as { content?: string }).content),
    ).toEqual(['{"id":1}', '{"id":2}', '{"id":3}']);
  });
});

describe('工具超时与截断', () => {
  it('超时产生错误结果并中止工具信号', async () => {
    vi.useFakeTimers();
    let toolSignalAborted = false;
    const { provider, requests } = createScriptedProvider([
      toolCallResponse([toolCall('c1', 'slow', {})]),
      textResponse('已恢复'),
    ]);
    const tool = createTool({
      name: 'slow',
      description: '慢工具',
      inputSchema: {},
      execute: async (_input, context) => {
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
    const agent = createAgent({
      provider,
      model: 'm',
      tools: [tool],
      toolTimeoutMs: 50,
    });

    const promise = agent.run([createUserMessage('go')]);
    await vi.advanceTimersByTimeAsync(60);
    await promise;

    expect(toolSignalAborted).toBe(true);
    expect(requests[1]?.messages.at(-1)?.content[0]).toMatchObject({
      content: '工具执行超时（>50ms）',
      isError: true,
    });
  });

  it('工具级 timeoutMs 覆盖 Agent 级配置', async () => {
    vi.useFakeTimers();
    const { provider } = createScriptedProvider([
      toolCallResponse([toolCall('c1', 'slow', {})]),
      textResponse('ok'),
    ]);
    const tool = createTool({
      name: 'slow',
      description: '慢工具',
      inputSchema: {},
      timeoutMs: 10,
      execute: () => new Promise<string>(() => undefined),
    });
    const agent = createAgent({
      provider,
      model: 'm',
      tools: [tool],
      toolTimeoutMs: 5_000,
    });

    const promise = agent.run([createUserMessage('go')]);
    await vi.advanceTimersByTimeAsync(20);
    const result = await promise;

    expect(result.stopReason).toBe(AgentStopReason.Completed);
    expect(result.generatedMessages[1]?.content[0]).toMatchObject({
      content: '工具执行超时（>10ms）',
      isError: true,
    });
  });

  it('超长工具结果被截断并标记', async () => {
    const { provider } = createScriptedProvider([
      toolCallResponse([toolCall('c1', 'long', {})]),
      textResponse('ok'),
    ]);
    const tool = createTool({
      name: 'long',
      description: '长结果',
      inputSchema: {},
      execute: () => '123456789012345678901234567890',
    });
    const agent = createAgent({
      provider,
      model: 'm',
      tools: [tool],
      maxToolResultLength: 20,
    });

    const result = await agent.run([createUserMessage('go')]);

    const toolResult = result.generatedMessages[1]?.content[0];
    expect(toolResult).toMatchObject({
      content: '12345\n...[truncated]',
      isError: false,
    });
    if (toolResult?.type === 'tool_result') {
      expect(toolResult.content).toHaveLength(20);
    }
  });
});

describe('运行期覆盖', () => {
  it('maxSteps 和 shouldStream 可在单次运行覆盖', async () => {
    const { provider, counts } = createStreamingProvider(
      [
        {
          type: 'response',
          response: toolCallResponse([toolCall('c1', 'echo', {})]),
        },
      ],
      toolCallResponse([toolCall('c1', 'echo', {})]),
    );
    const agent = createAgent({
      provider,
      model: 'm',
      tools: [echoTool()],
      shouldStream: false,
      maxSteps: 5,
    });

    const result = await agent.run([createUserMessage('go')], {
      shouldStream: true,
      maxSteps: 1,
    });

    expect(counts.stream).toBe(1);
    expect(counts.generate).toBe(0);
    expect(result.stopReason).toBe(AgentStopReason.MaxSteps);
    expect(result.steps).toBe(1);
  });

  it('非法 signal 在运行前被拒绝', async () => {
    const { provider } = createScriptedProvider([textResponse('ok')]);
    const agent = createAgent({ provider, model: 'm' });

    await expect(
      agent.run([createUserMessage('go')], { signal: {} as AbortSignal }),
    ).rejects.toBeInstanceOf(AgentDefinitionError);
  });

  it('toolChoice 引用未知工具时构造期失败', () => {
    const { provider } = createScriptedProvider([]);

    expect(() =>
      createAgent({
        provider,
        model: 'm',
        tools: [echoTool()],
        toolChoice: { type: 'tool', name: 'missing' },
      }),
    ).toThrow(AgentDefinitionError);
  });

  it('required 需要至少一个工具', () => {
    const { provider } = createScriptedProvider([]);

    expect(() => createAgent({ provider, model: 'm', toolChoice: 'required' })).toThrow(
      AgentDefinitionError,
    );
  });
});
