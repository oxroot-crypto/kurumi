/**
 * @file 数据提供方 HTTP 工具。
 * @description 统一 fetch 注入、超时、AbortSignal、JSON 解析与错误包装；不绑定具体数据源。
 * @module finance/provider/http
 */

import { ProviderDataError, ProviderHttpError } from './errors.js';

/** 默认超时（毫秒）。 */
const DEFAULT_TIMEOUT_MS = 15_000;

/** AbortSignal.timeout 支持的最大延迟。 */
const MAX_TIMEOUT_MS = 2_147_483_647;

/** 错误响应体最大读取字节数。 */
const MAX_ERROR_BODY_BYTES = 4_096;

/** 可注入的 fetch 实现。 */
export type FetchLike = (input: string, init?: RequestInit) => Promise<Response>;

/** JSON 请求选项。 */
export interface FetchJsonOptions {
  /** 提供方 id。 */
  readonly providerId: string;
  /** 可注入 fetch；缺省使用全局 fetch。 */
  readonly fetchImpl?: FetchLike;
  /** 请求头。 */
  readonly headers?: Readonly<Record<string, string>>;
  /** 超时毫秒；缺省 15_000。 */
  readonly timeoutMs?: number;
  /** 取消信号。 */
  readonly signal?: AbortSignal;
}

/**
 * 发起 GET 请求并解析 JSON。
 *
 * @param url - 请求 URL
 * @param options - 提供方 id、fetch、headers、超时与取消信号
 * @returns 解析后的 JSON
 * @throws {ProviderHttpError} 网络失败、超时或非 2xx
 * @throws {ProviderDataError} JSON 解析失败
 */
export async function fetchJson(url: string, options: FetchJsonOptions): Promise<unknown> {
  const response = await requestResponse(url, options);
  try {
    return await response.json();
  } catch (cause) {
    throw new ProviderDataError(options.providerId, '响应不是合法 JSON', cause);
  }
}

/**
 * 发起 GET 请求并读取文本。
 *
 * @param url - 请求 URL
 * @param options - 提供方 id、fetch、headers、超时与取消信号
 * @returns 响应文本
 * @throws {ProviderHttpError} 网络失败、超时或非 2xx
 */
export async function fetchText(url: string, options: FetchJsonOptions): Promise<string> {
  const response = await requestResponse(url, options);
  try {
    return await response.text();
  } catch (cause) {
    throw new ProviderDataError(options.providerId, '响应文本读取失败', cause);
  }
}

/** 发起请求并校验 HTTP 状态。 */
async function requestResponse(url: string, options: FetchJsonOptions): Promise<Response> {
  const fetchImpl = options.fetchImpl ?? fetch;
  const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs <= 0 || timeoutMs > MAX_TIMEOUT_MS) {
    throw new ProviderDataError(
      options.providerId,
      `timeoutMs 必须是 1..${MAX_TIMEOUT_MS} 之间的安全整数（实际值: ${String(timeoutMs)}）`,
    );
  }
  const timeoutSignal = AbortSignal.timeout(timeoutMs);
  const signal =
    options.signal === undefined ? timeoutSignal : AbortSignal.any([options.signal, timeoutSignal]);
  let response: Response;
  try {
    response = await fetchImpl(url, {
      method: 'GET',
      headers: { accept: 'application/json, text/plain, */*', ...options.headers },
      signal,
    });
  } catch (cause) {
    throw new ProviderHttpError(
      options.providerId,
      url,
      0,
      signal.aborted ? '请求已取消或超时' : '网络请求失败',
      cause,
    );
  }
  if (!response.ok) {
    const detail = await readErrorDetail(response);
    throw new ProviderHttpError(
      options.providerId,
      url,
      response.status,
      detail === '' ? response.statusText : detail,
    );
  }
  return response;
}

/** 流式读取错误响应体，最多 4 KiB，避免恶意响应造成内存放大。 */
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
      if (value.byteLength > remaining) {
        return `${result}…`;
      }
    }
    return result + decoder.decode();
  } catch {
    return '';
  } finally {
    try {
      await reader.cancel();
    } catch {
      // 已消费完或底层关闭时 cancel 可能失败，不覆盖原始 HTTP 错误。
    }
    try {
      reader.releaseLock();
    } catch {
      // release 失败不应覆盖原始 HTTP 错误。
    }
  }
}
