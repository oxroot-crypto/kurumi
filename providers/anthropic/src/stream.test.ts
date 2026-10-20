/**
 * @file Anthropic 协议适配器流式测试。
 * @description 用假 fetch 构造 SSE 响应，验证增量、最终组装、请求映射与错误。
 * @module anthropic/stream.test
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

import { AnthropicProviderError, createAnthropicProvider } from './index.js';

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

/** 将事件数组编码为 Anthropic SSE 响应。 */
function sseResponse(events: readonly Record<string, unknown>[], shouldFinish = true): Response {
  const lines: string[] = [];
  for (const event of events) {
    lines.push(`event: ${String(event['type'])}\n`);
    lines.push(`data: ${JSON.stringify(event)}\n\n`);
  }
  if (shouldFinish) {
    lines.push('event: message_stop\ndata: {"type":"message_stop"}\n\n');
  }
  return new Response(lines.join(''), {
    status: 200,
    headers: { 'content-type': 'text/event-stream' },
  });
}

/** 构造 message_start 事件。 */
function messageStart(usage: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    type: 'message_start',
    message: {
      id: 'msg_1',
      type: 'message',
      role: 'assistant',
      content: [],
      stop_reason: null,
      usage: { input_tokens: 10, output_tokens: 0, ...usage },
    },
  };
}

/** 构造 message_delta 事件。 */
function messageDelta(
  stopReason: string,
  usage: Record<string, unknown> = { output_tokens: 2 },
): Record<string, unknown> {
  return {
    type: 'message_delta',
    delta: { stop_reason: stopReason, stop_sequence: null },
    usage,
  };
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
  it('发送 stream=true 与 Anthropic 请求头', async () => {
    const { fetchImpl, calls } = fakeFetch(() => sseResponse([]));
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    await streamEvents(provider);

    const call = calls[0] as FetchCall;
    expect(call.url).toBe('https://api.anthropic.com/v1/messages');
    expect(bodyOf(call)['stream']).toBe(true);
    expect(new Headers(call.init.headers).get('x-api-key')).toBe('key');
  });

  it('传递 AbortSignal 给 fetch', async () => {
    const { fetchImpl, calls } = fakeFetch(() => sseResponse([]));
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });
    const controller = new AbortController();

    await streamEvents(provider, { signal: controller.signal });

    expect(calls[0]?.init.signal).toBe(controller.signal);
  });
});

describe('流式增量与最终响应', () => {
  it('转发文本增量并汇总 usage', async () => {
    const { fetchImpl } = fakeFetch(() =>
      sseResponse([
        messageStart({ cache_creation_input_tokens: 2, cache_read_input_tokens: 1 }),
        {
          type: 'content_block_start',
          index: 0,
          content_block: { type: 'text', text: '' },
        },
        { type: 'ping' },
        {
          type: 'content_block_delta',
          index: 0,
          delta: { type: 'text_delta', text: '你' },
        },
        {
          type: 'content_block_delta',
          index: 0,
          delta: { type: 'text_delta', text: '好' },
        },
        { type: 'content_block_stop', index: 0 },
        messageDelta('end_turn', { input_tokens: 10, output_tokens: 2 }),
      ]),
    );
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    const events = await streamEvents(provider);
    const deltas = events.filter((event) => event.type !== 'response');
    const final = events.at(-1);

    expect(deltas).toEqual([
      { type: 'text_delta', text: '你' },
      { type: 'text_delta', text: '好' },
    ]);
    expect(final?.type).toBe('response');
    if (final?.type === 'response') {
      expect(extractMessageText(final.response.message)).toBe('你好');
      expect(final.response.stopReason).toBe(ModelStopReason.End);
      expect(final.response.usage).toEqual({
        inputTokens: 13,
        outputTokens: 2,
        totalTokens: 15,
        cacheReadTokens: 1,
        cacheWriteTokens: 2,
      });
    }
  });

  it('转发 thinking 与 text，并保留内容块顺序', async () => {
    const { fetchImpl } = fakeFetch(() =>
      sseResponse([
        messageStart(),
        {
          type: 'content_block_start',
          index: 0,
          content_block: { type: 'thinking', thinking: '' },
        },
        {
          type: 'content_block_delta',
          index: 0,
          delta: { type: 'thinking_delta', thinking: '思考' },
        },
        {
          type: 'content_block_delta',
          index: 0,
          delta: { type: 'signature_delta', signature: 'sig' },
        },
        { type: 'content_block_stop', index: 0 },
        {
          type: 'content_block_start',
          index: 1,
          content_block: { type: 'text', text: '' },
        },
        {
          type: 'content_block_delta',
          index: 1,
          delta: { type: 'text_delta', text: 'ok' },
        },
        { type: 'content_block_stop', index: 1 },
        messageDelta('end_turn'),
      ]),
    );
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    const events = await streamEvents(provider);
    const final = events.at(-1);

    expect(events[0]).toEqual({ type: 'reasoning_delta', text: '思考' });
    expect(final?.type).toBe('response');
    if (final?.type === 'response') {
      expect(final.response.message.content).toEqual([
        { type: 'reasoning', text: '思考', signature: 'sig' },
        { type: 'text', text: 'ok' },
      ]);
    }
  });

  it('累积多个 signature_delta 后写入 reasoning', async () => {
    const { fetchImpl } = fakeFetch(() =>
      sseResponse([
        messageStart(),
        {
          type: 'content_block_start',
          index: 0,
          content_block: { type: 'thinking', thinking: '' },
        },
        {
          type: 'content_block_delta',
          index: 0,
          delta: { type: 'thinking_delta', thinking: '思考' },
        },
        {
          type: 'content_block_delta',
          index: 0,
          delta: { type: 'signature_delta', signature: 'sig-' },
        },
        {
          type: 'content_block_delta',
          index: 0,
          delta: { type: 'signature_delta', signature: '1' },
        },
        messageDelta('end_turn'),
      ]),
    );
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    const events = await streamEvents(provider);
    const final = events.at(-1);

    if (final?.type === 'response') {
      expect(final.response.message.content).toEqual([
        { type: 'reasoning', text: '思考', signature: 'sig-1' },
        { type: 'text', text: '' },
      ]);
    }
  });

  it('映射 redacted_thinking 起始块', async () => {
    const { fetchImpl } = fakeFetch(() =>
      sseResponse([
        messageStart(),
        {
          type: 'content_block_start',
          index: 0,
          content_block: { type: 'redacted_thinking', data: 'opaque-data' },
        },
        {
          type: 'content_block_start',
          index: 1,
          content_block: { type: 'text', text: '回答' },
        },
        messageDelta('end_turn'),
      ]),
    );
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    const events = await streamEvents(provider);
    const final = events.at(-1);

    if (final?.type === 'response') {
      expect(final.response.message.content).toEqual([
        { type: 'reasoning', text: '', redactedData: 'opaque-data' },
        { type: 'text', text: '回答' },
      ]);
    }
  });

  it('content_block_start 自带非空文本/思考时立即转发', async () => {
    const { fetchImpl } = fakeFetch(() =>
      sseResponse([
        messageStart(),
        {
          type: 'content_block_start',
          index: 0,
          content_block: { type: 'thinking', thinking: '初始思考' },
        },
        {
          type: 'content_block_start',
          index: 1,
          content_block: { type: 'text', text: '初始文本' },
        },
        messageDelta('end_turn'),
      ]),
    );
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    const events = await streamEvents(provider);

    expect(events[0]).toEqual({ type: 'reasoning_delta', text: '初始思考' });
    expect(events[1]).toEqual({ type: 'text_delta', text: '初始文本' });
  });

  it('转发工具参数增量并组装最终工具调用', async () => {
    const { fetchImpl } = fakeFetch(() =>
      sseResponse([
        messageStart({ input_tokens: 5 }),
        {
          type: 'content_block_start',
          index: 0,
          content_block: { type: 'tool_use', id: 'c1', name: 'search', input: {} },
        },
        {
          type: 'content_block_delta',
          index: 0,
          delta: { type: 'input_json_delta', partial_json: '{"q":' },
        },
        {
          type: 'content_block_delta',
          index: 0,
          delta: { type: 'input_json_delta', partial_json: '"x"}' },
        },
        { type: 'content_block_stop', index: 0 },
        messageDelta('tool_use', { output_tokens: 3 }),
      ]),
    );
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    const events = await streamEvents(provider);
    const deltas = events.filter((event) => event.type !== 'response');
    const final = events.at(-1);

    expect(deltas[0]).toEqual({
      type: 'tool_call_delta',
      index: 0,
      callId: 'c1',
      name: 'search',
      argumentsDelta: '',
    });
    expect(deltas.slice(1)).toEqual([
      { type: 'tool_call_delta', index: 0, argumentsDelta: '{"q":' },
      { type: 'tool_call_delta', index: 0, argumentsDelta: '"x"}' },
    ]);
    if (final?.type === 'response') {
      expect(getToolCalls(final.response.message)[0]).toMatchObject({
        callId: 'c1',
        name: 'search',
        input: { q: 'x' },
      });
      expect(final.response.stopReason).toBe(ModelStopReason.ToolCalls);
      expect(final.response.usage).toMatchObject({ inputTokens: 5, outputTokens: 3 });
    } else {
      expect.unreachable('缺少最终响应');
    }
  });

  it('多个内容块按 index 顺序组装，空工具参数视为对象', async () => {
    const { fetchImpl } = fakeFetch(() =>
      sseResponse([
        messageStart(),
        {
          type: 'content_block_start',
          index: 2,
          content_block: { type: 'tool_use', id: 'c1', name: 'search', input: {} },
        },
        {
          type: 'content_block_start',
          index: 0,
          content_block: { type: 'text', text: '先' },
        },
        {
          type: 'content_block_start',
          index: 1,
          content_block: { type: 'thinking', thinking: '思考' },
        },
        messageDelta('end_turn'),
      ]),
    );
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    const events = await streamEvents(provider);
    const final = events.at(-1);

    if (final?.type === 'response') {
      expect(final.response.message.content).toEqual([
        { type: 'text', text: '先' },
        { type: 'reasoning', text: '思考' },
        { type: 'tool_call', callId: 'c1', name: 'search', input: {} },
      ]);
      expect(final.response.stopReason).toBe(ModelStopReason.ToolCalls);
    } else {
      expect.unreachable('缺少最终响应');
    }
  });

  it('没有 message_stop 时仍产生最终响应', async () => {
    const { fetchImpl } = fakeFetch(() =>
      sseResponse(
        [
          messageStart(),
          {
            type: 'content_block_start',
            index: 0,
            content_block: { type: 'text', text: 'ok' },
          },
        ],
        false,
      ),
    );
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    const events = await streamEvents(provider);

    expect(events.at(-1)?.type).toBe('response');
  });

  it('空流产生空文本响应', async () => {
    const { fetchImpl } = fakeFetch(() => sseResponse([], false));
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    const events = await streamEvents(provider);
    const final = events.at(-1);

    if (final?.type === 'response') {
      expect(final.response.message.content).toEqual([{ type: 'text', text: '' }]);
      expect(final.response.stopReason).toBe(ModelStopReason.Other);
      expect(final.response.usage).toBeUndefined();
    } else {
      expect.unreachable('缺少最终响应');
    }
  });

  it('支持跨读取边界与 CRLF 的 SSE', async () => {
    const payload = [
      'event: message_start',
      `data: ${JSON.stringify(messageStart())}`,
      '',
      'event: content_block_start',
      'data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}',
      '',
      'event: content_block_delta',
      'data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"ok"}}',
      '',
      'event: message_delta',
      `data: ${JSON.stringify(messageDelta('end_turn'))}`,
      '',
      '',
    ].join('\r\n');
    const midpoint = Math.floor(payload.length / 2);
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(new TextEncoder().encode(payload.slice(0, midpoint)));
        controller.enqueue(new TextEncoder().encode(payload.slice(midpoint)));
        controller.close();
      },
    });
    const { fetchImpl } = fakeFetch(
      () => new Response(body, { status: 200, headers: { 'content-type': 'text/event-stream' } }),
    );
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    const events = await streamEvents(provider);
    const final = events.at(-1);

    if (final?.type === 'response') {
      expect(extractMessageText(final.response.message)).toBe('ok');
    } else {
      expect.unreachable('缺少最终响应');
    }
  });
});

describe('流式错误', () => {
  it('HTTP 错误抛适配器错误', async () => {
    const { fetchImpl } = fakeFetch(() => new Response('rate limited', { status: 429 }));
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    await expect(streamEvents(provider)).rejects.toMatchObject({
      name: 'AnthropicProviderError',
      status: 429,
    });
  });

  it('缺少 body 抛适配器错误', async () => {
    const { fetchImpl } = fakeFetch(() => new Response(null, { status: 200 }));
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    await expect(streamEvents(provider)).rejects.toBeInstanceOf(AnthropicProviderError);
  });

  it('读取流时底层报错包装为适配器错误', async () => {
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(new TextEncoder().encode('data: {"type":"ping"}\n\n'));
        controller.error(new Error('network broken'));
      },
    });
    const { fetchImpl } = fakeFetch(
      () => new Response(body, { status: 200, headers: { 'content-type': 'text/event-stream' } }),
    );
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    await expect(streamEvents(provider)).rejects.toMatchObject({
      name: 'AnthropicProviderError',
      message: expect.stringContaining('读取流失败'),
    });
  });

  it('SSE error payload 必须抛适配器错误且遮蔽 API Key', async () => {
    const { fetchImpl } = fakeFetch(() =>
      sseResponse(
        [
          {
            type: 'error',
            error: { type: 'overloaded_error', message: 'boom super-secret' },
          },
        ],
        false,
      ),
    );
    const provider = createAnthropicProvider({ apiKey: 'super-secret', fetch: fetchImpl });

    try {
      await streamEvents(provider);
      expect.unreachable('应当抛错');
    } catch (error) {
      expect(error).toBeInstanceOf(AnthropicProviderError);
      expect((error as Error).message).toContain('[REDACTED]');
      expect((error as Error).message).not.toContain('super-secret');
    }
  });

  it('SSE error payload 没有消息时使用通用错误', async () => {
    const { fetchImpl } = fakeFetch(() => sseResponse([{ type: 'error', error: {} }], false));
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    await expect(streamEvents(provider)).rejects.toMatchObject({
      message: expect.stringContaining('流返回错误'),
    });
  });

  it('SSE JSON 非法抛适配器错误', async () => {
    const { fetchImpl } = fakeFetch(
      () =>
        new Response('data: {bad\n\n', {
          status: 200,
          headers: { 'content-type': 'text/event-stream' },
        }),
    );
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    await expect(streamEvents(provider)).rejects.toBeInstanceOf(AnthropicProviderError);
  });

  it('未知事件类型抛适配器错误', async () => {
    const { fetchImpl } = fakeFetch(() => sseResponse([{ type: 'mystery' }], false));
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    await expect(streamEvents(provider)).rejects.toMatchObject({
      message: expect.stringContaining('未知流事件类型'),
    });
  });

  it('缺少 type 的 SSE 事件抛适配器错误', async () => {
    const { fetchImpl } = fakeFetch(
      () =>
        new Response('data: {"foo":1}\n\n', {
          status: 200,
          headers: { 'content-type': 'text/event-stream' },
        }),
    );
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    await expect(streamEvents(provider)).rejects.toBeInstanceOf(AnthropicProviderError);
  });

  it.each([
    ['message 非对象', { type: 'message_start', message: 1 }],
    ['message.usage 非对象', { type: 'message_start', message: { usage: 1 } }],
    ['content_block 非对象', { type: 'content_block_start', index: 0, content_block: 1 }],
    [
      'content_block.type 非字符串',
      { type: 'content_block_start', index: 0, content_block: { type: 1 } },
    ],
    [
      'content_block.index 为负',
      { type: 'content_block_start', index: -1, content_block: { type: 'text' } },
    ],
    [
      'content_block.text 非字符串',
      {
        type: 'content_block_start',
        index: 0,
        content_block: { type: 'text', text: 1 },
      },
    ],
    [
      'content_block.thinking 非字符串',
      {
        type: 'content_block_start',
        index: 0,
        content_block: { type: 'thinking', thinking: 1 },
      },
    ],
    [
      'content_block.signature 非字符串',
      {
        type: 'content_block_start',
        index: 0,
        content_block: { type: 'thinking', signature: 1 },
      },
    ],
    [
      'redacted_thinking 缺少 data',
      {
        type: 'content_block_start',
        index: 0,
        content_block: { type: 'redacted_thinking' },
      },
    ],
    [
      '工具缺少 id',
      {
        type: 'content_block_start',
        index: 0,
        content_block: { type: 'tool_use', name: 'x', input: {} },
      },
    ],
    [
      '工具 id 为空',
      {
        type: 'content_block_start',
        index: 0,
        content_block: { type: 'tool_use', id: ' ', name: 'x', input: {} },
      },
    ],
    [
      '工具 name 非法',
      {
        type: 'content_block_start',
        index: 0,
        content_block: { type: 'tool_use', id: 'c1', name: '1x', input: {} },
      },
    ],
    [
      '工具起始 input 非空',
      {
        type: 'content_block_start',
        index: 0,
        content_block: { type: 'tool_use', id: 'c1', name: 'x', input: { q: 1 } },
      },
    ],
    [
      '工具起始 input 非对象',
      {
        type: 'content_block_start',
        index: 0,
        content_block: { type: 'tool_use', id: 'c1', name: 'x', input: 1 },
      },
    ],
    ['未知内容块类型', { type: 'content_block_start', index: 0, content_block: { type: 'image' } }],
    ['delta 非对象', { type: 'content_block_delta', index: 0, delta: 1 }],
    ['delta.type 非字符串', { type: 'content_block_delta', index: 0, delta: { type: 1 } }],
    ['未知 delta 类型', { type: 'content_block_delta', index: 0, delta: { type: 'mystery' } }],
    ['message_delta.delta 非对象', { type: 'message_delta', delta: 1 }],
    ['message_delta.stop_reason 非字符串', { type: 'message_delta', delta: { stop_reason: 1 } }],
    [
      'message_delta.usage 非对象',
      { type: 'message_delta', delta: { stop_reason: 'end_turn' }, usage: 1 },
    ],
    [
      'message_delta.usage 为负',
      {
        type: 'message_delta',
        delta: { stop_reason: 'end_turn' },
        usage: { output_tokens: -1 },
      },
    ],
  ])('拒绝%s', async (_name, event) => {
    const { fetchImpl } = fakeFetch(() => sseResponse([event], false));
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    await expect(streamEvents(provider)).rejects.toBeInstanceOf(AnthropicProviderError);
  });

  it.each([
    [
      '没有对应内容块的 delta',
      [{ type: 'content_block_delta', index: 9, delta: { type: 'text_delta', text: 'x' } }],
    ],
    ['没有对应内容块的 stop', [{ type: 'content_block_stop', index: 9 }]],
    [
      '重复的 index',
      [
        { type: 'content_block_start', index: 0, content_block: { type: 'text' } },
        { type: 'content_block_start', index: 0, content_block: { type: 'text' } },
      ],
    ],
    [
      'delta 与内容块类型不匹配',
      [
        { type: 'content_block_start', index: 0, content_block: { type: 'text' } },
        {
          type: 'content_block_delta',
          index: 0,
          delta: { type: 'input_json_delta', partial_json: '{}' },
        },
      ],
    ],
    [
      'text_delta 文本非字符串',
      [
        { type: 'content_block_start', index: 0, content_block: { type: 'text' } },
        {
          type: 'content_block_delta',
          index: 0,
          delta: { type: 'text_delta', text: 1 },
        },
      ],
    ],
    [
      'thinking_delta 文本非字符串',
      [
        { type: 'content_block_start', index: 0, content_block: { type: 'thinking' } },
        {
          type: 'content_block_delta',
          index: 0,
          delta: { type: 'thinking_delta', thinking: 1 },
        },
      ],
    ],
    [
      'input_json_delta 非字符串',
      [
        {
          type: 'content_block_start',
          index: 0,
          content_block: { type: 'tool_use', id: 'c1', name: 'x', input: {} },
        },
        {
          type: 'content_block_delta',
          index: 0,
          delta: { type: 'input_json_delta', partial_json: 1 },
        },
      ],
    ],
    [
      'signature_delta 非字符串',
      [
        { type: 'content_block_start', index: 0, content_block: { type: 'thinking' } },
        {
          type: 'content_block_delta',
          index: 0,
          delta: { type: 'signature_delta', signature: 1 },
        },
      ],
    ],
  ])('拒绝%s', async (_name, events) => {
    const { fetchImpl } = fakeFetch(() => sseResponse(events, false));
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    await expect(streamEvents(provider)).rejects.toBeInstanceOf(AnthropicProviderError);
  });

  it('工具参数非法 JSON 在最终响应阶段抛错', async () => {
    const { fetchImpl } = fakeFetch(() =>
      sseResponse(
        [
          messageStart(),
          {
            type: 'content_block_start',
            index: 0,
            content_block: { type: 'tool_use', id: 'c1', name: 'x', input: {} },
          },
          {
            type: 'content_block_delta',
            index: 0,
            delta: { type: 'input_json_delta', partial_json: '{' },
          },
          { type: 'content_block_stop', index: 0 },
          messageDelta('tool_use'),
        ],
        false,
      ),
    );
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    await expect(streamEvents(provider)).rejects.toBeInstanceOf(AnthropicProviderError);
  });

  it('超过最大缓冲长度的事件抛错', async () => {
    const { fetchImpl } = fakeFetch(
      () =>
        new Response(`data: {"type":"ping","pad":"${'x'.repeat(1_048_576)}"}`, {
          status: 200,
          headers: { 'content-type': 'text/event-stream' },
        }),
    );
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    await expect(streamEvents(provider)).rejects.toMatchObject({
      message: expect.stringContaining('最大长度'),
    });
  });
});
