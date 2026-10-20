/**
 * @file OpenAI 协议适配器：选项、非流式请求/响应与错误测试。
 * @description 使用假 fetch 验证协议映射，不访问真实网络。
 * @module open-ai/provider.test
 */

import { describe, expect, it } from 'vitest';

import {
  ModelStopReason,
  createAssistantMessage,
  createSystemMessage,
  createToolMessage,
  createUserMessage,
  extractMessageText,
  getToolCalls,
} from '@kurumi/agent';

import { OpenAIProviderError, createOpenAIProvider, type OpenAIProviderOptions } from './index.js';

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

describe('选项', () => {
  it('默认 id 与 OpenAI 根地址', () => {
    const { fetchImpl } = fakeFetch(() => jsonResponse({}));
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });

    expect(provider.id).toBe('open-ai');
  });

  it('支持自定义 id、根地址与请求头', async () => {
    const { fetchImpl, calls } = fakeFetch(() =>
      jsonResponse({
        choices: [{ message: { content: 'ok' }, finish_reason: 'stop' }],
      }),
    );
    const provider = createOpenAIProvider({
      id: 'deepseek',
      apiKey: 'key',
      baseUrl: 'https://api.deepseek.com///',
      headers: { 'x-trace': 'abc', Authorization: 'evil', 'Content-Type': 'text/plain' },
      fetch: fetchImpl,
    });

    await provider.generate({ model: 'm', messages: [createUserMessage('hi')], tools: [] }, {});

    expect(provider.id).toBe('deepseek');
    expect(calls[0]?.url).toBe('https://api.deepseek.com/chat/completions');
    expect(new Headers(calls[0]?.init.headers).get('x-trace')).toBe('abc');
    expect(new Headers(calls[0]?.init.headers).get('authorization')).toBe('Bearer key');
    expect(new Headers(calls[0]?.init.headers).get('content-type')).toBe('application/json');
  });

  it.each([
    ['选项为 null', null],
    ['apiKey 为空', { apiKey: ' ' }],
    ['id 为空', { apiKey: 'key', id: ' ' }],
    ['baseUrl 为空', { apiKey: 'key', baseUrl: ' ' }],
    ['baseUrl 不是 URL', { apiKey: 'key', baseUrl: 'not a url' }],
    ['baseUrl 协议非法', { apiKey: 'key', baseUrl: 'file:///tmp/api' }],
    ['baseUrl 带查询参数', { apiKey: 'key', baseUrl: 'https://api.example.com/v1?x=1' }],
    ['headers 非对象', { apiKey: 'key', headers: [] }],
    ['headers 值非字符串', { apiKey: 'key', headers: { x: 1 } }],
    ['fetch 非函数', { apiKey: 'key', fetch: 1 }],
    ['maxTokensField 非法', { apiKey: 'key', maxTokensField: 'max_output_tokens' }],
    ['shouldIncludeStreamUsage 非布尔', { apiKey: 'key', shouldIncludeStreamUsage: 1 }],
    ['reasoningContentField 为空', { apiKey: 'key', reasoningContentField: ' ' }],
    ['thinkingField 为空', { apiKey: 'key', thinkingField: ' ' }],
    ['thinkingField 非字符串', { apiKey: 'key', thinkingField: 1 }],
    ['thinkingField 使用保留字段', { apiKey: 'key', thinkingField: 'reasoning_effort' }],
  ])('拒绝%s', (_name, value) => {
    expect(() => createOpenAIProvider(value as OpenAIProviderOptions)).toThrow(OpenAIProviderError);
  });
});

describe('非流式请求映射', () => {
  it('映射消息、图片、工具、工具选择与透传选项', async () => {
    const { fetchImpl, calls } = fakeFetch(() =>
      jsonResponse({
        choices: [{ message: { content: 'ok' }, finish_reason: 'stop' }],
      }),
    );
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });
    const messages = [
      createSystemMessage('sys'),
      createUserMessage([
        { type: 'text', text: '看图' },
        { type: 'image', mediaType: 'image/png', data: 'abc' },
      ]),
      createAssistantMessage([
        { type: 'reasoning', text: '思考' },
        { type: 'tool_call', callId: 'c1', name: 'search', input: { q: 'x' } },
        { type: 'text', text: '稍后' },
      ]),
      createToolMessage({ callId: 'c1', name: 'search', content: '结果' }),
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
          model: 'evil',
          stream: true,
          max_tokens: 1,
          temperature: 9,
        },
      },
      {},
    );

    const body = bodyOf(calls[0] as FetchCall);
    expect(body['model']).toBe('m');
    expect(body['stream']).toBe(false);
    expect(body['temperature']).toBe(0.3);
    expect(body['max_tokens']).toBe(128);
    expect(body['top_p']).toBe(0.9);
    expect(body['tool_choice']).toEqual({ type: 'function', function: { name: 'search' } });
    expect(body['tools']).toEqual([
      {
        type: 'function',
        function: {
          name: 'search',
          description: '搜索',
          parameters: { type: 'object', properties: { q: { type: 'string' } } },
        },
      },
    ]);

    const sent = body['messages'] as Array<Record<string, unknown>>;
    expect(sent[0]).toEqual({ role: 'system', content: 'sys' });
    expect(sent[1]).toEqual({
      role: 'user',
      content: [
        { type: 'text', text: '看图' },
        { type: 'image_url', image_url: { url: 'data:image/png;base64,abc' } },
      ],
    });
    expect(sent[2]).toEqual({
      role: 'assistant',
      content: '稍后',
      tool_calls: [
        {
          id: 'c1',
          type: 'function',
          function: { name: 'search', arguments: '{"q":"x"}' },
        },
      ],
    });
    expect(sent[3]).toEqual({
      role: 'tool',
      content: '结果',
      tool_call_id: 'c1',
      name: 'search',
    });
  });

  it('映射 thinking 与 effort，并优先于 providerOptions', async () => {
    const { fetchImpl, calls } = fakeFetch(() =>
      jsonResponse({ choices: [{ message: { content: 'ok' }, finish_reason: 'stop' }] }),
    );
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });

    await provider.generate(
      {
        model: 'm',
        messages: [createUserMessage('hi')],
        tools: [],
        thinking: { isEnabled: true, effort: 'high', budgetTokens: 1_024 },
        providerOptions: {
          thinking: { type: 'disabled' },
          reasoning_effort: 'low',
        },
      },
      {},
    );

    const body = bodyOf(calls[0] as FetchCall);
    expect(body['thinking']).toEqual({ type: 'enabled' });
    expect(body['reasoning_effort']).toBe('high');
  });

  it('thinking=false 发送 disabled，thinkingField=false 时不发送开关', async () => {
    const { fetchImpl, calls } = fakeFetch(() =>
      jsonResponse({ choices: [{ message: { content: 'ok' }, finish_reason: 'stop' }] }),
    );
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });
    const reasoningOnly = createOpenAIProvider({
      apiKey: 'key',
      fetch: fetchImpl,
      thinkingField: false,
    });

    await provider.generate(
      {
        model: 'm',
        messages: [createUserMessage('hi')],
        tools: [],
        thinking: { isEnabled: false },
      },
      {},
    );
    await reasoningOnly.generate(
      {
        model: 'm',
        messages: [createUserMessage('hi')],
        tools: [],
        thinking: { isEnabled: true, effort: 'medium' },
      },
      {},
    );

    expect(bodyOf(calls[0] as FetchCall)['thinking']).toEqual({ type: 'disabled' });
    expect(bodyOf(calls[1] as FetchCall)).not.toHaveProperty('thinking');
    expect(bodyOf(calls[1] as FetchCall)['reasoning_effort']).toBe('medium');
  });

  it('未配置 thinking 时保留 providerOptions 中的思考字段', async () => {
    const { fetchImpl, calls } = fakeFetch(() =>
      jsonResponse({ choices: [{ message: { content: 'ok' }, finish_reason: 'stop' }] }),
    );
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });

    await provider.generate(
      {
        model: 'm',
        messages: [createUserMessage('hi')],
        tools: [],
        providerOptions: {
          thinking: { type: 'enabled' },
          reasoning_effort: 'low',
        },
      },
      {},
    );

    const body = bodyOf(calls[0] as FetchCall);
    expect(body['thinking']).toEqual({ type: 'enabled' });
    expect(body['reasoning_effort']).toBe('low');
  });

  it('纯文本 user 使用字符串，assistant 工具调用无文本时 content=null', async () => {
    const { fetchImpl, calls } = fakeFetch(() =>
      jsonResponse({ choices: [{ message: { content: 'ok' }, finish_reason: 'stop' }] }),
    );
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });

    await provider.generate(
      {
        model: 'm',
        messages: [
          createUserMessage('纯文本'),
          createAssistantMessage([{ type: 'tool_call', callId: 'c1', name: 'echo', input: {} }]),
        ],
        tools: [],
        toolChoice: 'auto',
      },
      {},
    );

    const sent = bodyOf(calls[0] as FetchCall)['messages'] as Array<Record<string, unknown>>;
    expect(sent[0]).toEqual({ role: 'user', content: '纯文本' });
    expect(sent[1]).toEqual({
      role: 'assistant',
      content: null,
      tool_calls: [{ id: 'c1', type: 'function', function: { name: 'echo', arguments: '{}' } }],
    });
  });

  it('maxTokensField 可切换为 max_completion_tokens', async () => {
    const { fetchImpl, calls } = fakeFetch(() =>
      jsonResponse({ choices: [{ message: { content: 'ok' }, finish_reason: 'stop' }] }),
    );
    const provider = createOpenAIProvider({
      apiKey: 'key',
      fetch: fetchImpl,
      maxTokensField: 'max_completion_tokens',
    });

    await provider.generate(
      { model: 'm', messages: [createUserMessage('hi')], tools: [], maxOutputTokens: 64 },
      {},
    );

    const body = bodyOf(calls[0] as FetchCall);
    expect(body['max_completion_tokens']).toBe(64);
    expect(body).not.toHaveProperty('max_tokens');
  });

  it('没有工具时不发送 tools 与 tool_choice', async () => {
    const { fetchImpl, calls } = fakeFetch(() =>
      jsonResponse({ choices: [{ message: { content: 'ok' }, finish_reason: 'stop' }] }),
    );
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });

    await provider.generate({ model: 'm', messages: [createUserMessage('hi')], tools: [] }, {});

    const body = bodyOf(calls[0] as FetchCall);
    expect(body).not.toHaveProperty('tools');
    expect(body).not.toHaveProperty('tool_choice');
  });
});

describe('非流式响应映射', () => {
  it('映射文本、推理、工具调用、缓存 usage 与停止原因', async () => {
    const { fetchImpl } = fakeFetch(() =>
      jsonResponse({
        choices: [
          {
            message: {
              content: '回答',
              reasoning_content: '推理',
              tool_calls: [
                {
                  id: 'c1',
                  type: 'function',
                  function: { name: 'search', arguments: '{"q":"x"}' },
                },
              ],
            },
            finish_reason: 'tool_calls',
          },
        ],
        usage: {
          prompt_tokens: 10,
          completion_tokens: 5,
          total_tokens: 15,
          prompt_tokens_details: { cached_tokens: 4 },
        },
      }),
    );
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });

    const response = await provider.generate(
      { model: 'm', messages: [createUserMessage('hi')], tools: [] },
      {},
    );

    expect(extractMessageText(response.message)).toBe('回答');
    expect(response.message.content[0]).toEqual({ type: 'reasoning', text: '推理' });
    expect(getToolCalls(response.message)[0]).toMatchObject({
      callId: 'c1',
      name: 'search',
      input: { q: 'x' },
    });
    expect(response.stopReason).toBe(ModelStopReason.ToolCalls);
    expect(response.usage).toEqual({
      inputTokens: 10,
      outputTokens: 5,
      totalTokens: 15,
      cacheReadTokens: 4,
    });
  });

  it('支持 DeepSeek prompt_cache_hit_tokens', async () => {
    const { fetchImpl } = fakeFetch(() =>
      jsonResponse({
        choices: [{ message: { content: 'ok' }, finish_reason: 'stop' }],
        usage: { prompt_tokens: 3, completion_tokens: 2, prompt_cache_hit_tokens: 1 },
      }),
    );
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });

    const response = await provider.generate(
      { model: 'm', messages: [createUserMessage('hi')], tools: [] },
      {},
    );

    expect(response.usage).toMatchObject({ cacheReadTokens: 1, totalTokens: 5 });
  });

  it.each([
    ['stop', false, ModelStopReason.End],
    ['stop', true, ModelStopReason.ToolCalls],
    ['length', false, ModelStopReason.MaxTokens],
    ['content_filter', false, ModelStopReason.ContentFilter],
    ['tool_calls', true, ModelStopReason.ToolCalls],
    ['unknown', false, ModelStopReason.Other],
    ['unknown', true, ModelStopReason.ToolCalls],
  ])('finish_reason=%s hasTools=%s -> %s', async (reason, hasTools, expected) => {
    const toolCalls = hasTools
      ? [{ id: 'c1', type: 'function', function: { name: 'echo', arguments: '{}' } }]
      : undefined;
    const { fetchImpl } = fakeFetch(() =>
      jsonResponse({
        choices: [{ message: { content: 'ok', tool_calls: toolCalls }, finish_reason: reason }],
      }),
    );
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });

    const response = await provider.generate(
      { model: 'm', messages: [createUserMessage('hi')], tools: [] },
      {},
    );

    expect(response.stopReason).toBe(expected);
  });

  it('内容为空时生成空文本块', async () => {
    const { fetchImpl } = fakeFetch(() =>
      jsonResponse({ choices: [{ message: { content: null }, finish_reason: 'stop' }] }),
    );
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });

    const response = await provider.generate(
      { model: 'm', messages: [createUserMessage('hi')], tools: [] },
      {},
    );

    expect(response.message.content).toEqual([{ type: 'text', text: '' }]);
  });

  it('空参数视为对象', async () => {
    const { fetchImpl } = fakeFetch(() =>
      jsonResponse({
        choices: [
          {
            message: {
              tool_calls: [
                { id: 'c1', type: 'function', function: { name: 'echo', arguments: '' } },
              ],
            },
            finish_reason: 'tool_calls',
          },
        ],
      }),
    );
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });

    const response = await provider.generate(
      { model: 'm', messages: [createUserMessage('hi')], tools: [] },
      {},
    );

    expect(getToolCalls(response.message)[0]?.input).toEqual({});
  });

  it.each([
    ['非对象响应', null],
    ['缺少 choices', {}],
    ['缺少 message', { choices: [{}] }],
    ['content 非字符串', { choices: [{ message: { content: 1 }, finish_reason: 'stop' }] }],
    ['tool_calls 非数组', { choices: [{ message: { tool_calls: 1 }, finish_reason: 'stop' }] }],
    [
      '工具缺少 id',
      {
        choices: [
          {
            message: {
              tool_calls: [{ type: 'function', function: { name: 'x', arguments: '{}' } }],
            },
            finish_reason: 'tool_calls',
          },
        ],
      },
    ],
    [
      '工具参数非法 JSON',
      {
        choices: [
          {
            message: {
              tool_calls: [{ id: 'c1', type: 'function', function: { name: 'x', arguments: '{' } }],
            },
            finish_reason: 'tool_calls',
          },
        ],
      },
    ],
    [
      'usage 非法',
      {
        choices: [{ message: { content: 'x' }, finish_reason: 'stop' }],
        usage: { prompt_tokens: -1, completion_tokens: 1 },
      },
    ],
    [
      'usage 非对象',
      {
        choices: [{ message: { content: 'x' }, finish_reason: 'stop' }],
        usage: 1,
      },
    ],
  ])('拒绝%s', async (_name, payload) => {
    const { fetchImpl } = fakeFetch(() => jsonResponse(payload));
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });

    await expect(
      provider.generate({ model: 'm', messages: [createUserMessage('hi')], tools: [] }, {}),
    ).rejects.toBeInstanceOf(OpenAIProviderError);
  });

  it('HTTP 错误包含状态且不泄漏 API Key', async () => {
    const { fetchImpl } = fakeFetch(() => new Response('bad key', { status: 401 }));
    const provider = createOpenAIProvider({ apiKey: 'super-secret', fetch: fetchImpl });

    try {
      await provider.generate({ model: 'm', messages: [createUserMessage('hi')], tools: [] }, {});
      expect.unreachable('应当抛错');
    } catch (error) {
      expect(error).toBeInstanceOf(OpenAIProviderError);
      expect((error as OpenAIProviderError).status).toBe(401);
      expect((error as OpenAIProviderError).code).toBe('open_ai_provider_error');
      expect((error as Error).message).toContain('HTTP 401');
      expect((error as Error).message).not.toContain('super-secret');
    }
  });

  it('超大 HTTP 错误体只读取有限前缀并截断消息', async () => {
    const { fetchImpl } = fakeFetch(() => new Response('x'.repeat(100_000), { status: 500 }));
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });

    try {
      await provider.generate({ model: 'm', messages: [createUserMessage('hi')], tools: [] }, {});
      expect.unreachable('应当抛错');
    } catch (error) {
      expect(error).toBeInstanceOf(OpenAIProviderError);
      expect((error as Error).message.length).toBeLessThan(2_000);
    }
  });

  it('网络/传输错误包装为适配器错误并保留 cause', async () => {
    const { fetchImpl } = fakeFetch(async () => {
      throw new TypeError('network down');
    });
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });

    try {
      await provider.generate({ model: 'm', messages: [createUserMessage('hi')], tools: [] }, {});
      expect.unreachable('应当抛错');
    } catch (error) {
      expect(error).toBeInstanceOf(OpenAIProviderError);
      expect((error as OpenAIProviderError).cause).toBeInstanceOf(TypeError);
      expect((error as Error).message).toContain('网络请求失败');
    }
  });

  it('非 JSON 响应抛适配器错误', async () => {
    const { fetchImpl } = fakeFetch(() => new Response('not json', { status: 200 }));
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });

    await expect(
      provider.generate({ model: 'm', messages: [createUserMessage('hi')], tools: [] }, {}),
    ).rejects.toBeInstanceOf(OpenAIProviderError);
  });

  it('传递 AbortSignal 给 fetch', async () => {
    const { fetchImpl, calls } = fakeFetch(() =>
      jsonResponse({ choices: [{ message: { content: 'ok' }, finish_reason: 'stop' }] }),
    );
    const provider = createOpenAIProvider({ apiKey: 'key', fetch: fetchImpl });
    const controller = new AbortController();

    await provider.generate(
      { model: 'm', messages: [createUserMessage('hi')], tools: [] },
      { signal: controller.signal },
    );

    expect(calls[0]?.init.signal).toBe(controller.signal);
  });
});
