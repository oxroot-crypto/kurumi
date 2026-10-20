/**
 * @file OpenAI 协议适配器流式测试。
 * @description 用假 fetch 构造 SSE 响应，验证增量、最终组装、请求映射与错误。
 * @module open-ai/stream.test
 */

import { describe, expect, it } from 'vitest';

import {
  ModelStopReason,
  createUserMessage,
  extractMessageText,
  getToolCalls,
  type ModelCallOptions,
  type ModelProvider,
  type ModelRequest,
  type ModelStreamEvent,
} from '@kurumi/agent';

import { OpenAIProviderError, createOpenAIProvider } from './index.js';

/** 捕获的 fetch 调用。 */
interface FetchCall {
  readonly url: string;
  readonly init: RequestInit;
}

/** 创建假 fetch。 */
function fakeFetch(responder: (url: string, init: RequestInit) => Response | Promise<Response>): {
  readonly fetchImpl: typeof fetch;
  readonly calls: FetchCall[];
} {
  const calls: FetchCall[] = [];
  const fetchImpl: typeof fetch = async (input, init) => {
    const requestInit = init ?? {};
    calls.push({ url: String(input), init: requestInit });
    return responder(String(input), requestInit);
  };
  return { fetchImpl, calls };
}

/** 将 chunk 数组编码为 SSE 响应。 */
function sseResponse(chunks: readonly unknown[], shouldFinish = true): Response {
  const lines = chunks.map((chunk) => `data: ${JSON.stringify(chunk)}\n\n`);
  if (shouldFinish) lines.push('data: [DONE]\n\n');
  return new Response(lines.join(''), {
    status: 200,
    headers: { 'content-type': 'text/event-stream' },
  });
}

/** 收集流事件。 */
async function collect(events: AsyncIterable<ModelStreamEvent>): Promise<ModelStreamEvent[]> {
  const result: ModelStreamEvent[] = [];
  for await (const event of events) result.push(event);
  return result;
}

/** 构造请求。 */
function request(): ModelRequest {
  return { model: 'm', messages: [createUserMessage('hi')], tools: [] };
}

/** 调用提供方流式接口并收集事件。 */
async function streamEvents(
  provider: ModelProvider,
  callOptions: ModelCallOptions = {},
): Promise<ModelStreamEvent[]> {
  const stream = provider.stream;
  if (stream === undefined) throw new Error('提供方未实现 stream');
  return collect(stream(request(), callOptions));
}

/** 读取请求体。 */
function bodyOf(call: FetchCall): Record<string, unknown> {
  if (typeof call.init.body !== 'string') throw new Error('请求体不是字符串');
  return JSON.parse(call.init.body) as Record<string, unknown>;
}

describe('流式请求', () => {
  it('发送 stream=true 与 stream_options', async () => {
    const { fetchImpl, calls } = fakeFetch(() =>
      sseResponse([{ choices: [{ delta: { content: 'ok' }, finish_reason: 'stop' }] }]),
    );
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });

    await streamEvents(provider);

    const body = bodyOf(calls[0] as FetchCall);
    expect(body['stream']).toBe(true);
    expect(body['stream_options']).toEqual({ include_usage: true });
  });

  it('可关闭 stream_options', async () => {
    const { fetchImpl, calls } = fakeFetch(() =>
      sseResponse([{ choices: [{ delta: { content: 'ok' }, finish_reason: 'stop' }] }]),
    );
    const provider = createOpenAIProvider({
      apiKey: 'key',
      fetch: fetchImpl,
      shouldIncludeStreamUsage: false,
    });

    await streamEvents(provider);

    expect(bodyOf(calls[0] as FetchCall)).not.toHaveProperty('stream_options');
  });

  it('传递 AbortSignal 给 fetch', async () => {
    const { fetchImpl, calls } = fakeFetch(() =>
      sseResponse([{ choices: [{ delta: { content: 'ok' }, finish_reason: 'stop' }] }]),
    );
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });
    const controller = new AbortController();

    await streamEvents(provider, { signal: controller.signal });

    expect(calls[0]?.init.signal).toBe(controller.signal);
  });
});

describe('流式增量与最终响应', () => {
  it('转发文本、推理、工具参数并组装最终响应', async () => {
    const { fetchImpl } = fakeFetch(() =>
      sseResponse([
        { choices: [{ delta: { content: '回答' } }] },
        { choices: [{ delta: { reasoning_content: '推理' } }] },
        {
          choices: [
            {
              delta: {
                tool_calls: [
                  {
                    index: 0,
                    id: 'c1',
                    type: 'function',
                    function: { name: 'search', arguments: '{"q":' },
                  },
                ],
              },
            },
          ],
        },
        {
          choices: [
            {
              delta: {
                tool_calls: [{ index: 0, function: { arguments: '"x"}' } }],
              },
            },
          ],
        },
        { choices: [{ delta: {}, finish_reason: 'tool_calls' }] },
        {
          choices: [],
          usage: {
            prompt_tokens: 10,
            completion_tokens: 5,
            prompt_tokens_details: { cached_tokens: 2 },
          },
        },
      ]),
    );
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });

    const events = await streamEvents(provider);
    const deltas = events.filter((event) => event.type !== 'response');
    const final = events.at(-1);

    expect(deltas.map((event) => event.type)).toEqual([
      'text_delta',
      'reasoning_delta',
      'tool_call_delta',
      'tool_call_delta',
    ]);
    expect(deltas[2]).toMatchObject({
      type: 'tool_call_delta',
      index: 0,
      callId: 'c1',
      name: 'search',
      argumentsDelta: '{"q":',
    });
    expect(final?.type).toBe('response');
    if (final?.type === 'response') {
      expect(extractMessageText(final.response.message)).toBe('回答');
      expect(final.response.message.content[0]).toEqual({ type: 'reasoning', text: '推理' });
      expect(getToolCalls(final.response.message)[0]).toMatchObject({
        callId: 'c1',
        name: 'search',
        input: { q: 'x' },
      });
      expect(final.response.stopReason).toBe(ModelStopReason.ToolCalls);
      expect(final.response.usage).toEqual({
        inputTokens: 10,
        outputTokens: 5,
        totalTokens: 15,
        cacheReadTokens: 2,
      });
    }
  });

  it('自定义推理字段名', async () => {
    const { fetchImpl } = fakeFetch(() =>
      sseResponse([
        { choices: [{ delta: { reasoning: '思考' } }] },
        { choices: [{ delta: { content: 'ok' }, finish_reason: 'stop' }] },
      ]),
    );
    const provider = createOpenAIProvider({
      apiKey: 'key',
      fetch: fetchImpl,
      reasoningContentField: 'reasoning',
    });

    const events = await streamEvents(provider);

    expect(events[0]).toEqual({ type: 'reasoning_delta', text: '思考' });
  });

  it('没有 [DONE] 时仍产生最终响应', async () => {
    const { fetchImpl } = fakeFetch(() =>
      sseResponse([{ choices: [{ delta: { content: 'ok' }, finish_reason: 'stop' }] }], false),
    );
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });

    const events = await streamEvents(provider);

    expect(events.at(-1)?.type).toBe('response');
  });

  it('空 choices 与 null usage 被忽略', async () => {
    const { fetchImpl } = fakeFetch(() =>
      sseResponse([
        { choices: [], usage: null },
        { choices: [{ delta: { content: '' }, finish_reason: 'stop' }] },
      ]),
    );
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });

    const events = await streamEvents(provider);
    const final = events.at(-1);

    expect(final?.type).toBe('response');
    if (final?.type === 'response') {
      expect(final.response.message.content).toEqual([{ type: 'text', text: '' }]);
    }
  });

  it('finish_reason=length 映射为 max_tokens', async () => {
    const { fetchImpl } = fakeFetch(() =>
      sseResponse([{ choices: [{ delta: { content: 'x' }, finish_reason: 'length' }] }]),
    );
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });

    const events = await streamEvents(provider);
    const final = events.at(-1);

    expect(final?.type).toBe('response');
    if (final?.type === 'response') {
      expect(final.response.stopReason).toBe(ModelStopReason.MaxTokens);
    }
  });
});

describe('流式错误', () => {
  it('HTTP 错误抛适配器错误', async () => {
    const { fetchImpl } = fakeFetch(() => new Response('rate limited', { status: 429 }));
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });

    await expect(streamEvents(provider)).rejects.toMatchObject({
      name: 'OpenAIProviderError',
      status: 429,
    });
  });

  it('缺少 body 抛适配器错误', async () => {
    const { fetchImpl } = fakeFetch(() => new Response(null, { status: 200 }));
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });

    await expect(streamEvents(provider)).rejects.toBeInstanceOf(OpenAIProviderError);
  });

  it('SSE JSON 非法抛适配器错误', async () => {
    const { fetchImpl } = fakeFetch(
      () =>
        new Response('data: {bad\n\n', {
          status: 200,
          headers: { 'content-type': 'text/event-stream' },
        }),
    );
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });

    await expect(streamEvents(provider)).rejects.toBeInstanceOf(OpenAIProviderError);
  });

  it('SSE error payload 必须抛适配器错误而不是返回空响应', async () => {
    const { fetchImpl } = fakeFetch(() => sseResponse([{ error: { message: 'boom' } }]));
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });

    await expect(streamEvents(provider)).rejects.toMatchObject({
      name: 'OpenAIProviderError',
      message: expect.stringContaining('boom'),
    });
  });

  it.each([
    ['choices 元素非对象', { choices: ['bad'] }],
    ['delta 非对象', { choices: [{ delta: 'bad' }] }],
    ['缺少 choices 且无 usage', { foo: 'bar' }],
  ])('拒绝%s', async (_name, chunk) => {
    const { fetchImpl } = fakeFetch(() => sseResponse([chunk]));
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });

    await expect(streamEvents(provider)).rejects.toBeInstanceOf(OpenAIProviderError);
  });

  it('读取流时底层报错包装为适配器错误', async () => {
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(new TextEncoder().encode('data: {"choices":[]}\n\n'));
        controller.error(new Error('network broken'));
      },
    });
    const { fetchImpl } = fakeFetch(
      () => new Response(body, { status: 200, headers: { 'content-type': 'text/event-stream' } }),
    );
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });

    await expect(streamEvents(provider)).rejects.toMatchObject({
      name: 'OpenAIProviderError',
      message: expect.stringContaining('读取流失败'),
    });
  });

  it.each([
    ['content 非字符串', { choices: [{ delta: { content: 1 } }] }],
    ['tool_calls 非数组', { choices: [{ delta: { tool_calls: 1 } }] }],
    ['usage 非对象', { choices: [], usage: 1 }],
    ['choices 非数组', { choices: 1 }],
    [
      'index 为负',
      { choices: [{ delta: { tool_calls: [{ index: -1, function: { arguments: '' } }] } }] },
    ],
    [
      'callId 为空',
      { choices: [{ delta: { tool_calls: [{ index: 0, id: '', function: { arguments: '' } }] } }] },
    ],
    [
      'name 为空',
      {
        choices: [
          {
            delta: {
              tool_calls: [{ index: 0, id: 'c1', function: { name: '', arguments: '' } }],
            },
          },
        ],
      },
    ],
    [
      'arguments 非字符串',
      {
        choices: [
          {
            delta: {
              tool_calls: [{ index: 0, id: 'c1', function: { name: 'x', arguments: 1 } }],
            },
          },
        ],
      },
    ],
  ])('拒绝%s', async (_name, chunk) => {
    const { fetchImpl } = fakeFetch(() => sseResponse([chunk]));
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });

    await expect(streamEvents(provider)).rejects.toBeInstanceOf(OpenAIProviderError);
  });

  it('工具参数非法 JSON 在最终响应阶段抛错', async () => {
    const { fetchImpl } = fakeFetch(() =>
      sseResponse([
        {
          choices: [
            {
              delta: {
                tool_calls: [
                  {
                    index: 0,
                    id: 'c1',
                    function: { name: 'x', arguments: '{"q":' },
                  },
                ],
              },
              finish_reason: 'tool_calls',
            },
          ],
        },
      ]),
    );
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });

    await expect(streamEvents(provider)).rejects.toBeInstanceOf(OpenAIProviderError);
  });
});
