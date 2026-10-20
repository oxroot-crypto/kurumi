/**
 * @file Anthropic 协议模型提供方工厂。
 * @description 负责 HTTP 调用、错误包装与非流式/流式分流；不写日志。
 * @module anthropic/provider
 */

import type {
  ModelCallOptions,
  ModelProvider,
  ModelRequest,
  ModelResponse,
  ModelStreamEvent,
} from '@kurumi/agent';

import { AnthropicProviderError, redactAnthropicSecret } from './errors.js';
import {
  resolveAnthropicProviderOptions,
  type AnthropicProviderOptions,
  type ResolvedAnthropicProviderOptions,
} from './options.js';
import { toHeaders, toRequestBody } from './request.js';
import { toModelResponse } from './response.js';
import { consumeStream } from './stream.js';

/** 错误详情最大字符数。 */
const MAX_ERROR_DETAIL_LENGTH = 1_000;

/** 读取错误响应体的最大字节数，避免恶意/异常响应造成内存放大。 */
const MAX_ERROR_BODY_BYTES = 16 * 1_024;

/**
 * 创建 Anthropic Messages 协议兼容的模型提供方。
 *
 * @param options - API Key、根地址、headers、fetch 与协议扩展选项
 * @returns 实现 `generate` 与 `stream` 的冻结提供方
 * @throws {AnthropicProviderError} 选项非法
 *
 * @example
 * ```ts
 * const provider = createAnthropicProvider({
 *   id: 'deepseek-anthropic',
 *   apiKey: process.env.KURUMI_API_KEY!,
 *   baseUrl: 'https://api.deepseek.com/anthropic',
 * });
 * ```
 */
export function createAnthropicProvider(options: AnthropicProviderOptions): ModelProvider {
  const settings = resolveAnthropicProviderOptions(options);

  async function generate(
    request: ModelRequest,
    callOptions: ModelCallOptions,
  ): Promise<ModelResponse> {
    const response = await post(settings, request, false, callOptions.signal);
    const payload = await readJson(settings, response);
    return toModelResponse(settings, payload);
  }

  async function* stream(
    request: ModelRequest,
    callOptions: ModelCallOptions,
  ): AsyncGenerator<ModelStreamEvent, void, void> {
    const response = await post(settings, request, true, callOptions.signal);
    yield* consumeStream(response, settings);
  }

  return Object.freeze({ id: settings.id, generate, stream });
}

/** 发送 HTTP 请求。 */
async function post(
  options: ResolvedAnthropicProviderOptions,
  request: ModelRequest,
  shouldStream: boolean,
  signal: AbortSignal | undefined,
): Promise<Response> {
  let response: Response;
  try {
    response = await options.fetchImpl(options.messagesUrl, {
      method: 'POST',
      headers: toHeaders(options),
      body: JSON.stringify(toRequestBody(request, options, shouldStream)),
      ...(signal === undefined ? {} : { signal }),
    });
  } catch (cause) {
    if (cause instanceof AnthropicProviderError) throw cause;
    throw new AnthropicProviderError(
      options.id,
      signal?.aborted === true ? '请求已取消' : '网络请求失败',
      { cause },
    );
  }
  if (!response.ok) {
    const detail = redactAnthropicSecret(await readErrorDetail(response), options.apiKey);
    throw new AnthropicProviderError(
      options.id,
      `HTTP ${String(response.status)}: ${truncate(detail, MAX_ERROR_DETAIL_LENGTH)}`,
      { status: response.status },
    );
  }
  return response;
}

/** 读取非流式 JSON。 */
async function readJson(
  options: ResolvedAnthropicProviderOptions,
  response: Response,
): Promise<unknown> {
  try {
    return await response.json();
  } catch (error) {
    throw new AnthropicProviderError(options.id, '响应不是合法 JSON', { cause: error });
  }
}

/** 读取错误详情；读取失败时返回空字符串，避免掩盖原始 HTTP 错误。 */
async function readErrorDetail(response: Response): Promise<string> {
  const body = response.body;
  if (body === null) return '';

  const reader = body.getReader();
  const decoder = new TextDecoder();
  let result = '';
  let bytesRead = 0;
  try {
    while (bytesRead < MAX_ERROR_BODY_BYTES) {
      const { done, value } = await reader.read();
      if (done === true) break;
      if (value === undefined) continue;
      const remaining = MAX_ERROR_BODY_BYTES - bytesRead;
      const chunk = value.byteLength > remaining ? value.subarray(0, remaining) : value;
      result += decoder.decode(chunk, { stream: true });
      bytesRead += chunk.byteLength;
      if (value.byteLength > remaining) break;
    }
    return result + decoder.decode();
  } catch {
    return '';
  } finally {
    try {
      await reader.cancel();
    } catch {
      // 已消费完或底层已关闭时 cancel 可能失败；不覆盖原始 HTTP 错误。
    }
    try {
      reader.releaseLock();
    } catch {
      // release 在异常流中也应尽力而为，不影响原始 HTTP 错误。
    }
  }
}

/** 截断错误详情。 */
function truncate(value: string, maxLength: number): string {
  if (value.length <= maxLength) return value;
  const suffix = '...';
  if (maxLength <= suffix.length) return value.slice(0, maxLength);
  return `${value.slice(0, maxLength - suffix.length)}${suffix}`;
}
