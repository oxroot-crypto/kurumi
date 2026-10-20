/**
 * @file Anthropic 协议适配器：选项、非流式请求/响应与错误测试。
 * @description 使用假 fetch 验证协议映射，不访问真实网络。
 * @module anthropic/provider.test
 */

import { describe, expect, it } from 'vitest';

import {
  ModelStopReason,
  createAssistantMessage,
  createSystemMessage,
  createToolMessage,
  createUserMessage,
  type AgentMessage,
  type AgentMessageRole,
  type ModelRequest,
} from '@kurumi/agent';

import {
  AnthropicProviderError,
  createAnthropicProvider,
  type AnthropicProviderOptions,
} from './index.js';

/** 捕获的 fetch 调用。 */
interface FetchCall {
  readonly url: string;
  readonly init: RequestInit;
}

/**
 * 创建假 fetch。
 *
 * @param responder - 根据 URL 与 init 返回响应
 * @returns fetch 与调用记录
 */
function fakeFetch(responder: (url: string, init: RequestInit) => Response | Promise<Response>): {
  readonly fetchImpl: typeof fetch;
  readonly calls: FetchCall[];
} {
  const calls: FetchCall[] = [];
  const fetchImpl: typeof fetch = async (input, init) => {
    const url = String(input);
    const requestInit = init ?? {};
    calls.push({ url, init: requestInit });
    return responder(url, requestInit);
  };
  return { fetchImpl, calls };
}

/** 构造 JSON 响应。 */
function jsonResponse(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

/** 读取请求体 JSON。 */
function bodyOf(call: FetchCall): Record<string, unknown> {
  const body = call.init.body;
  if (typeof body !== 'string') throw new Error('请求体不是字符串');
  return JSON.parse(body) as Record<string, unknown>;
}

/** 构造一个最小成功响应。 */
function successResponse(content: unknown[] = [{ type: 'text', text: 'ok' }]): Response {
  return jsonResponse({ type: 'message', role: 'assistant', content, stop_reason: 'end_turn' });
}

describe('选项', () => {
  it('默认 id、根地址与版本', async () => {
    const { fetchImpl, calls } = fakeFetch(() => successResponse());
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    await provider.generate({ model: 'm', messages: [createUserMessage('hi')], tools: [] }, {});

    expect(provider.id).toBe('anthropic');
    expect(calls[0]?.url).toBe('https://api.anthropic.com/v1/messages');
    expect(new Headers(calls[0]?.init.headers).get('x-api-key')).toBe('key');
    expect(new Headers(calls[0]?.init.headers).get('anthropic-version')).toBe('2023-06-01');
  });

  it('支持自定义 id、根地址、版本与请求头', async () => {
    const { fetchImpl, calls } = fakeFetch(() => successResponse());
    const provider = createAnthropicProvider({
      id: 'deepseek-anthropic',
      apiKey: 'key',
      baseUrl: 'https://api.deepseek.com/anthropic///',
      anthropicVersion: '2024-01-01',
      headers: {
        'anthropic-beta': 'files-api-2025-04-14',
        'x-api-key': 'evil',
        'Content-Type': 'text/plain',
        'anthropic-version': 'evil',
      },
      fetch: fetchImpl,
    });

    await provider.generate({ model: 'm', messages: [createUserMessage('hi')], tools: [] }, {});

    expect(provider.id).toBe('deepseek-anthropic');
    expect(calls[0]?.url).toBe('https://api.deepseek.com/anthropic/v1/messages');
    const headers = new Headers(calls[0]?.init.headers);
    expect(headers.get('anthropic-beta')).toBe('files-api-2025-04-14');
    expect(headers.get('x-api-key')).toBe('key');
    expect(headers.get('anthropic-version')).toBe('2024-01-01');
    expect(headers.get('content-type')).toBe('application/json');
  });

  it.each([
    ['选项为 null', null],
    ['apiKey 为空', { apiKey: ' ' }],
    ['id 为空', { apiKey: 'key', id: ' ' }],
    ['baseUrl 为空', { apiKey: 'key', baseUrl: ' ' }],
    ['baseUrl 不是 URL', { apiKey: 'key', baseUrl: 'not a url' }],
    ['baseUrl 协议非法', { apiKey: 'key', baseUrl: 'file:///tmp/api' }],
    ['baseUrl 带查询参数', { apiKey: 'key', baseUrl: 'https://api.example.com/v1?x=1' }],
    ['baseUrl 带 hash', { apiKey: 'key', baseUrl: 'https://api.example.com/v1#x' }],
    ['headers 非对象', { apiKey: 'key', headers: [] }],
    ['headers 值非字符串', { apiKey: 'key', headers: { x: 1 } }],
    ['fetch 非函数', { apiKey: 'key', fetch: 1 }],
    ['anthropicVersion 为空', { apiKey: 'key', anthropicVersion: ' ' }],
    ['defaultMaxTokens 为 0', { apiKey: 'key', defaultMaxTokens: 0 }],
    ['defaultMaxTokens 为负数', { apiKey: 'key', defaultMaxTokens: -1 }],
    ['defaultMaxTokens 为小数', { apiKey: 'key', defaultMaxTokens: 1.5 }],
    ['defaultMaxTokens 溢出', { apiKey: 'key', defaultMaxTokens: Number.MAX_SAFE_INTEGER + 1 }],
  ])('拒绝%s', (_name, value) => {
    expect(() => createAnthropicProvider(value as AnthropicProviderOptions)).toThrow(
      AnthropicProviderError,
    );
  });
});

describe('非流式请求映射', () => {
  it('映射 system、消息、图片、工具、工具选择与透传选项', async () => {
    const { fetchImpl, calls } = fakeFetch(() => successResponse());
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });
    const messages = [
      createSystemMessage('sys1'),
      createUserMessage([
        { type: 'text', text: '看图' },
        { type: 'image', mediaType: 'image/png', data: 'abc' },
      ]),
      createAssistantMessage([
        { type: 'reasoning', text: '思考' },
        { type: 'tool_call', callId: 'c1', name: 'search', input: { q: 'x' } },
        { type: 'text', text: '稍后' },
      ]),
      createToolMessage({ callId: 'c1', name: 'search', content: '结果1' }),
      createToolMessage({ callId: 'c2', name: 'search', content: '结果2', isError: true }),
      createSystemMessage('sys2'),
    ];

    await provider.generate(
      {
        model: 'm',
        messages,
        tools: [
          {
            name: 'search',
            description: '搜索',
            inputSchema: { type: 'object', properties: { q: { type: 'string' } } },
          },
        ],
        toolChoice: { type: 'tool', name: 'search' },
        temperature: 0.3,
        maxOutputTokens: 128,
        providerOptions: {
          top_p: 0.9,
          metadata: { user_id: 'u1' },
          model: 'evil',
          messages: [],
          system: 'evil',
          max_tokens: 1,
          stream: false,
          tools: [],
          tool_choice: { type: 'auto' },
          temperature: 1,
        },
      },
      {},
    );

    const body = bodyOf(calls[0] as FetchCall);
    expect(body['model']).toBe('m');
    expect(body['max_tokens']).toBe(128);
    expect(body['stream']).toBe(false);
    expect(body['system']).toBe('sys1\n\nsys2');
    expect(body['temperature']).toBe(0.3);
    expect(body['top_p']).toBe(0.9);
    expect(body['metadata']).toEqual({ user_id: 'u1' });
    expect(body['tools']).toEqual([
      {
        name: 'search',
        description: '搜索',
        input_schema: { type: 'object', properties: { q: { type: 'string' } } },
      },
    ]);
    expect(body['tool_choice']).toEqual({ type: 'tool', name: 'search' });

    const messagesBody = body['messages'] as readonly Record<string, unknown>[];
    expect(messagesBody).toHaveLength(3);
    expect(messagesBody[0]).toEqual({
      role: 'user',
      content: [
        { type: 'text', text: '看图' },
        {
          type: 'image',
          source: { type: 'base64', media_type: 'image/png', data: 'abc' },
        },
      ],
    });
    expect(messagesBody[1]).toEqual({
      role: 'assistant',
      content: [
        { type: 'tool_use', id: 'c1', name: 'search', input: { q: 'x' } },
        { type: 'text', text: '稍后' },
      ],
    });
    expect(messagesBody[2]).toEqual({
      role: 'user',
      content: [
        { type: 'tool_result', tool_use_id: 'c1', content: '结果1' },
        { type: 'tool_result', tool_use_id: 'c2', content: '结果2', is_error: true },
      ],
    });
  });

  it('映射 thinking 配置并优先于 providerOptions', async () => {
    const { fetchImpl, calls } = fakeFetch(() => successResponse());
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    await provider.generate(
      {
        model: 'm',
        messages: [createUserMessage('hi')],
        tools: [],
        thinking: { isEnabled: true, budgetTokens: 1_024, effort: 'high' },
        providerOptions: { thinking: { type: 'disabled' } },
      },
      {},
    );

    expect(bodyOf(calls[0] as FetchCall)['thinking']).toEqual({
      type: 'enabled',
      budget_tokens: 1_024,
    });
  });

  it('thinking=true 使用默认预算，false 发送 disabled', async () => {
    const { fetchImpl, calls } = fakeFetch(() => successResponse());
    const provider = createAnthropicProvider({
      apiKey: 'key',
      defaultThinkingBudgetTokens: 3_072,
      fetch: fetchImpl,
    });

    await provider.generate(
      {
        model: 'm',
        messages: [createUserMessage('hi')],
        tools: [],
        thinking: { isEnabled: true },
      },
      {},
    );
    await provider.generate(
      {
        model: 'm',
        messages: [createUserMessage('hi')],
        tools: [],
        thinking: { isEnabled: false },
      },
      {},
    );

    expect(bodyOf(calls[0] as FetchCall)['thinking']).toEqual({
      type: 'enabled',
      budget_tokens: 3_072,
    });
    expect(bodyOf(calls[1] as FetchCall)['thinking']).toEqual({ type: 'disabled' });
  });

  it('未配置 thinking 时保留 providerOptions.thinking', async () => {
    const { fetchImpl, calls } = fakeFetch(() => successResponse());
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    await provider.generate(
      {
        model: 'm',
        messages: [createUserMessage('hi')],
        tools: [],
        providerOptions: { thinking: { type: 'enabled', budget_tokens: 4_096 } },
      },
      {},
    );

    expect(bodyOf(calls[0] as FetchCall)['thinking']).toEqual({
      type: 'enabled',
      budget_tokens: 4_096,
    });
  });

  it('回传带签名与加密数据的 reasoning', async () => {
    const { fetchImpl, calls } = fakeFetch(() => successResponse());
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    await provider.generate(
      {
        model: 'm',
        messages: [
          createUserMessage('hi'),
          createAssistantMessage([
            { type: 'reasoning', text: '思考', signature: 'sig-1' },
            { type: 'reasoning', text: '', redactedData: 'opaque' },
            { type: 'reasoning', text: '无签名' },
            { type: 'text', text: '回答' },
          ]),
        ],
        tools: [],
      },
      {},
    );

    const messages = bodyOf(calls[0] as FetchCall)['messages'] as readonly Record<
      string,
      unknown
    >[];
    expect(messages[1]).toEqual({
      role: 'assistant',
      content: [
        { type: 'thinking', thinking: '思考', signature: 'sig-1' },
        { type: 'redacted_thinking', data: 'opaque' },
        { type: 'text', text: '回答' },
      ],
    });
  });

  it('纯文本 user 使用字符串内容', async () => {
    const { fetchImpl, calls } = fakeFetch(() => successResponse());
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    await provider.generate({ model: 'm', messages: [createUserMessage('hi')], tools: [] }, {});

    expect(bodyOf(calls[0] as FetchCall)['messages']).toEqual([{ role: 'user', content: 'hi' }]);
  });

  it.each([
    ['auto', { type: 'auto' }],
    ['none', { type: 'none' }],
    ['required', { type: 'any' }],
  ])('toolChoice=%s 映射为 %o', async (choice, expected) => {
    const { fetchImpl, calls } = fakeFetch(() => successResponse());
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    await provider.generate(
      {
        model: 'm',
        messages: [createUserMessage('hi')],
        tools: [{ name: 'search', description: '搜索', inputSchema: { type: 'object' } }],
        toolChoice: choice as 'auto' | 'none' | 'required',
      },
      {},
    );

    expect(bodyOf(calls[0] as FetchCall)['tool_choice']).toEqual(expected);
  });

  it('没有工具时不发送 tools 与 tool_choice', async () => {
    const { fetchImpl, calls } = fakeFetch(() => successResponse());
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    await provider.generate(
      {
        model: 'm',
        messages: [createUserMessage('hi')],
        tools: [],
        toolChoice: 'auto',
      },
      {},
    );

    const body = bodyOf(calls[0] as FetchCall);
    expect(body).not.toHaveProperty('tools');
    expect(body).not.toHaveProperty('tool_choice');
  });

  it('maxOutputTokens 缺省时使用 defaultMaxTokens', async () => {
    const { fetchImpl, calls } = fakeFetch(() => successResponse());
    const provider = createAnthropicProvider({
      apiKey: 'key',
      fetch: fetchImpl,
      defaultMaxTokens: 777,
    });

    await provider.generate({ model: 'm', messages: [createUserMessage('hi')], tools: [] }, {});

    expect(bodyOf(calls[0] as FetchCall)['max_tokens']).toBe(777);
  });

  it('空 system 不发送 system 字段', async () => {
    const { fetchImpl, calls } = fakeFetch(() => successResponse());
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    await provider.generate(
      { model: 'm', messages: [createSystemMessage(''), createUserMessage('hi')], tools: [] },
      {},
    );

    expect(bodyOf(calls[0] as FetchCall)).not.toHaveProperty('system');
  });

  it.each([
    ['模型为空', { model: ' ', messages: [createUserMessage('hi')], tools: [] }],
    [
      'maxOutputTokens 为 0',
      { model: 'm', messages: [createUserMessage('hi')], tools: [], maxOutputTokens: 0 },
    ],
    [
      'maxOutputTokens 为小数',
      { model: 'm', messages: [createUserMessage('hi')], tools: [], maxOutputTokens: 1.5 },
    ],
    [
      'temperature 为 NaN',
      { model: 'm', messages: [createUserMessage('hi')], tools: [], temperature: Number.NaN },
    ],
  ])('拒绝%s', async (_name, request) => {
    const { fetchImpl } = fakeFetch(() => successResponse());
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    await expect(provider.generate(request as ModelRequest, {})).rejects.toBeInstanceOf(
      AnthropicProviderError,
    );
  });

  it('请求为 null 时抛适配器错误', async () => {
    const { fetchImpl } = fakeFetch(() => successResponse());
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });
    const invalid: unknown = null;

    await expect(provider.generate(invalid as ModelRequest, {})).rejects.toBeInstanceOf(
      AnthropicProviderError,
    );
  });

  it('只有 system 或空消息数组时抛适配器错误', async () => {
    const { fetchImpl } = fakeFetch(() => successResponse());
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    await expect(
      provider.generate({ model: 'm', messages: [createSystemMessage('sys')], tools: [] }, {}),
    ).rejects.toBeInstanceOf(AnthropicProviderError);
    await expect(
      provider.generate({ model: 'm', messages: [], tools: [] }, {}),
    ).rejects.toBeInstanceOf(AnthropicProviderError);
  });

  it('assistant 只有 reasoning 时抛适配器错误', async () => {
    const { fetchImpl } = fakeFetch(() => successResponse());
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    await expect(
      provider.generate(
        {
          model: 'm',
          messages: [createAssistantMessage([{ type: 'reasoning', text: '只有思考' }])],
          tools: [],
        },
        {},
      ),
    ).rejects.toBeInstanceOf(AnthropicProviderError);
  });

  it('非法 tool 消息抛适配器错误', async () => {
    const { fetchImpl } = fakeFetch(() => successResponse());
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });
    const invalid: AgentMessage = { role: 'tool', content: [{ type: 'text', text: 'x' }] };

    await expect(
      provider.generate({ model: 'm', messages: [invalid], tools: [] }, {}),
    ).rejects.toBeInstanceOf(AnthropicProviderError);
  });

  it('非法消息角色抛适配器错误', async () => {
    const { fetchImpl } = fakeFetch(() => successResponse());
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });
    const invalid: AgentMessage = {
      role: 'developer' as AgentMessageRole,
      content: [{ type: 'text', text: 'x' }],
    };

    await expect(
      provider.generate({ model: 'm', messages: [invalid], tools: [] }, {}),
    ).rejects.toBeInstanceOf(AnthropicProviderError);
  });
});

describe('非流式响应映射', () => {
  it('按顺序映射 thinking、text 与 tool_use，并汇总 usage', async () => {
    const { fetchImpl } = fakeFetch(() =>
      jsonResponse({
        type: 'message',
        role: 'assistant',
        content: [
          { type: 'thinking', thinking: '思考', signature: 'sig' },
          { type: 'text', text: '回答' },
          { type: 'tool_use', id: 'c1', name: 'search', input: { q: 'x' } },
        ],
        stop_reason: 'tool_use',
        usage: {
          input_tokens: 10,
          output_tokens: 5,
          cache_creation_input_tokens: 2,
          cache_read_input_tokens: 4,
        },
      }),
    );
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    const response = await provider.generate(
      { model: 'm', messages: [createUserMessage('hi')], tools: [] },
      {},
    );

    expect(response.message.content).toEqual([
      { type: 'reasoning', text: '思考', signature: 'sig' },
      { type: 'text', text: '回答' },
      { type: 'tool_call', callId: 'c1', name: 'search', input: { q: 'x' } },
    ]);
    expect(response.stopReason).toBe(ModelStopReason.ToolCalls);
    expect(response.usage).toEqual({
      inputTokens: 16,
      outputTokens: 5,
      totalTokens: 21,
      cacheReadTokens: 4,
      cacheWriteTokens: 2,
    });
  });

  it('空内容生成空文本块', async () => {
    const { fetchImpl } = fakeFetch(() => successResponse([]));
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    const response = await provider.generate(
      { model: 'm', messages: [createUserMessage('hi')], tools: [] },
      {},
    );

    expect(response.message.content).toEqual([{ type: 'text', text: '' }]);
  });

  it('只有 thinking 时补空文本块，保证消息可回传', async () => {
    const { fetchImpl } = fakeFetch(() =>
      successResponse([{ type: 'thinking', thinking: '思考', signature: 'sig' }]),
    );
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    const response = await provider.generate(
      { model: 'm', messages: [createUserMessage('hi')], tools: [] },
      {},
    );

    expect(response.message.content).toEqual([
      { type: 'reasoning', text: '思考', signature: 'sig' },
      { type: 'text', text: '' },
    ]);
  });

  it('映射 redacted_thinking 并保留加密数据', async () => {
    const { fetchImpl } = fakeFetch(() =>
      successResponse([
        { type: 'redacted_thinking', data: 'opaque-data' },
        { type: 'text', text: '回答' },
      ]),
    );
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    const response = await provider.generate(
      { model: 'm', messages: [createUserMessage('hi')], tools: [] },
      {},
    );

    expect(response.message.content).toEqual([
      { type: 'reasoning', text: '', redactedData: 'opaque-data' },
      { type: 'text', text: '回答' },
    ]);
  });

  it.each([
    ['end_turn', false, ModelStopReason.End],
    ['stop_sequence', false, ModelStopReason.End],
    ['max_tokens', false, ModelStopReason.MaxTokens],
    ['tool_use', false, ModelStopReason.ToolCalls],
    ['refusal', false, ModelStopReason.ContentFilter],
    ['pause_turn', false, ModelStopReason.Other],
    [undefined, false, ModelStopReason.Other],
    ['end_turn', true, ModelStopReason.ToolCalls],
    ['max_tokens', true, ModelStopReason.MaxTokens],
  ])('stop_reason=%s hasTools=%s -> %s', async (reason, hasTools, expected) => {
    const content = hasTools
      ? [{ type: 'tool_use', id: 'c1', name: 'search', input: {} }]
      : [{ type: 'text', text: 'ok' }];
    const { fetchImpl } = fakeFetch(() =>
      jsonResponse({ content, ...(reason === undefined ? {} : { stop_reason: reason }) }),
    );
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    const response = await provider.generate(
      { model: 'm', messages: [createUserMessage('hi')], tools: [] },
      {},
    );

    expect(response.stopReason).toBe(expected);
  });

  it('usage 缺省或 null 时不返回 usage', async () => {
    const { fetchImpl } = fakeFetch(() => successResponse());
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });
    const request: ModelRequest = { model: 'm', messages: [createUserMessage('hi')], tools: [] };

    expect((await provider.generate(request, {})).usage).toBeUndefined();

    const { fetchImpl: nullUsageFetch } = fakeFetch(() =>
      jsonResponse({ content: [{ type: 'text', text: 'ok' }], usage: null }),
    );
    const nullUsageProvider = createAnthropicProvider({
      apiKey: 'key',
      fetch: nullUsageFetch,
    });
    expect((await nullUsageProvider.generate(request, {})).usage).toBeUndefined();
  });

  it.each([
    ['usage 非对象', { content: [], usage: 1 }],
    ['input_tokens 为负', { content: [], usage: { input_tokens: -1 } }],
    ['output_tokens 为小数', { content: [], usage: { output_tokens: 1.5 } }],
    [
      '输入总和溢出',
      {
        content: [],
        usage: { input_tokens: Number.MAX_SAFE_INTEGER, cache_read_input_tokens: 1 },
      },
    ],
  ])('拒绝%s', async (_name, payload) => {
    const { fetchImpl } = fakeFetch(() => jsonResponse(payload));
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    await expect(
      provider.generate({ model: 'm', messages: [createUserMessage('hi')], tools: [] }, {}),
    ).rejects.toBeInstanceOf(AnthropicProviderError);
  });

  it.each([
    ['非对象响应', null],
    ['响应 role 非法', { role: 'user', content: [] }],
    ['响应 type 非法', { type: 'error', content: [] }],
    ['缺少 content', {}],
    ['content 非数组', { content: 1 }],
    ['内容块非对象', { content: ['bad'] }],
    ['text 非字符串', { content: [{ type: 'text', text: 1 }] }],
    ['thinking 非字符串', { content: [{ type: 'thinking', thinking: 1 }] }],
    ['thinking 签名为空', { content: [{ type: 'thinking', thinking: 'x', signature: ' ' }] }],
    ['redacted_thinking 缺少 data', { content: [{ type: 'redacted_thinking' }] }],
    ['redacted_thinking data 为空', { content: [{ type: 'redacted_thinking', data: '' }] }],
    ['未知内容块', { content: [{ type: 'server_tool_use', id: 'x' }] }],
    ['工具缺少 id', { content: [{ type: 'tool_use', name: 'x', input: {} }] }],
    ['工具 id 为空', { content: [{ type: 'tool_use', id: ' ', name: 'x', input: {} }] }],
    ['工具 name 非法', { content: [{ type: 'tool_use', id: 'c1', name: '1x', input: {} }] }],
    ['工具 input 缺失', { content: [{ type: 'tool_use', id: 'c1', name: 'x' }] }],
    [
      '工具 input 非 JSON',
      { content: [{ type: 'tool_use', id: 'c1', name: 'x', input: () => 1 }] },
    ],
    ['stop_reason 非字符串', { content: [], stop_reason: 1 }],
  ])('拒绝%s', async (_name, payload) => {
    const { fetchImpl } = fakeFetch(() => jsonResponse(payload));
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    await expect(
      provider.generate({ model: 'm', messages: [createUserMessage('hi')], tools: [] }, {}),
    ).rejects.toBeInstanceOf(AnthropicProviderError);
  });
});

describe('HTTP 与传输错误', () => {
  it('HTTP 错误包含状态且不泄漏 API Key', async () => {
    const { fetchImpl } = fakeFetch(() => new Response('bad key super-secret', { status: 401 }));
    const provider = createAnthropicProvider({ apiKey: 'super-secret', fetch: fetchImpl });

    try {
      await provider.generate({ model: 'm', messages: [createUserMessage('hi')], tools: [] }, {});
      expect.unreachable('应当抛错');
    } catch (error) {
      expect(error).toBeInstanceOf(AnthropicProviderError);
      expect((error as AnthropicProviderError).status).toBe(401);
      expect((error as AnthropicProviderError).code).toBe('anthropic_provider_error');
      expect((error as Error).message).toContain('HTTP 401');
      expect((error as Error).message).not.toContain('super-secret');
    }
  });

  it('超大 HTTP 错误体只读取有限前缀并截断消息', async () => {
    const { fetchImpl } = fakeFetch(() => new Response('x'.repeat(100_000), { status: 500 }));
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    try {
      await provider.generate({ model: 'm', messages: [createUserMessage('hi')], tools: [] }, {});
      expect.unreachable('应当抛错');
    } catch (error) {
      expect(error).toBeInstanceOf(AnthropicProviderError);
      expect((error as Error).message.length).toBeLessThan(2_000);
    }
  });

  it('网络错误包装为适配器错误并保留 cause', async () => {
    const { fetchImpl } = fakeFetch(async () => {
      throw new TypeError('network down');
    });
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    try {
      await provider.generate({ model: 'm', messages: [createUserMessage('hi')], tools: [] }, {});
      expect.unreachable('应当抛错');
    } catch (error) {
      expect(error).toBeInstanceOf(AnthropicProviderError);
      expect((error as AnthropicProviderError).cause).toBeInstanceOf(TypeError);
      expect((error as Error).message).toContain('网络请求失败');
    }
  });

  it('取消时错误消息为请求已取消', async () => {
    const { fetchImpl } = fakeFetch(async () => {
      throw new TypeError('aborted');
    });
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });
    const controller = new AbortController();
    controller.abort();

    await expect(
      provider.generate(
        { model: 'm', messages: [createUserMessage('hi')], tools: [] },
        { signal: controller.signal },
      ),
    ).rejects.toMatchObject({ message: expect.stringContaining('请求已取消') });
  });

  it('非 JSON 响应抛适配器错误', async () => {
    const { fetchImpl } = fakeFetch(() => new Response('not json', { status: 200 }));
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });

    await expect(
      provider.generate({ model: 'm', messages: [createUserMessage('hi')], tools: [] }, {}),
    ).rejects.toBeInstanceOf(AnthropicProviderError);
  });

  it('传递 AbortSignal 给 fetch', async () => {
    const { fetchImpl, calls } = fakeFetch(() => successResponse());
    const provider = createAnthropicProvider({ apiKey: 'key', fetch: fetchImpl });
    const controller = new AbortController();

    await provider.generate(
      { model: 'm', messages: [createUserMessage('hi')], tools: [] },
      { signal: controller.signal },
    );

    expect(calls[0]?.init.signal).toBe(controller.signal);
  });
});
