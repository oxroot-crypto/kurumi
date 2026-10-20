/**
 * @file 适配器选项解析与校验。
 * @description 所有选项在构造期一次性校验并冻结，运行期不再读取调用方可变对象。
 * @module anthropic/options
 */

import { isPlainObject } from '@kurumi/core';

import { AnthropicProviderError } from './errors.js';

/** 默认 Anthropic API 根地址。 */
export const DEFAULT_ANTHROPIC_BASE_URL = 'https://api.anthropic.com';

/** 默认提供方 ID。 */
export const DEFAULT_ANTHROPIC_PROVIDER_ID = 'anthropic';

/** 默认 Messages API 版本。 */
export const DEFAULT_ANTHROPIC_VERSION = '2023-06-01';

/** 默认最大输出 Token 数；`max_tokens` 是 Messages API 的必填字段。 */
export const DEFAULT_ANTHROPIC_MAX_TOKENS = 4_096;

/** 默认思考预算 Token 数；仅在请求启用 thinking 且未提供预算时使用。 */
export const DEFAULT_ANTHROPIC_THINKING_BUDGET_TOKENS = 2_048;

/** Anthropic 协议适配器选项。 */
export interface AnthropicProviderOptions {
  /** API Key，必填；写入 `x-api-key`。 */
  readonly apiKey: string;
  /** API 根地址；缺省 `https://api.anthropic.com`，请求 `${baseUrl}/v1/messages`。 */
  readonly baseUrl?: string;
  /** 提供方 ID，缺省 `anthropic`。 */
  readonly id?: string;
  /** 追加请求头（例如 `anthropic-beta`）；不能覆盖 content-type、x-api-key、anthropic-version。 */
  readonly headers?: Readonly<Record<string, string>>;
  /** 可注入 fetch，便于测试或代理。 */
  readonly fetch?: typeof fetch;
  /** `anthropic-version` 请求头，缺省 `2023-06-01`。 */
  readonly anthropicVersion?: string;
  /** 未提供 `maxOutputTokens` 时使用的 `max_tokens`，缺省 4096。 */
  readonly defaultMaxTokens?: number;
  /** 启用 thinking 且未指定 `budgetTokens` 时使用的预算，缺省 2048。 */
  readonly defaultThinkingBudgetTokens?: number;
}

/** 解析后的适配器选项。 */
export interface ResolvedAnthropicProviderOptions {
  /** API Key。 */
  readonly apiKey: string;
  /** API 根地址（去尾斜杠）。 */
  readonly baseUrl: string;
  /** 提供方 ID。 */
  readonly id: string;
  /** 追加请求头（冻结副本）。 */
  readonly headers: Readonly<Record<string, string>>;
  /** fetch 实现。 */
  readonly fetchImpl: typeof fetch;
  /** `anthropic-version` 请求头。 */
  readonly anthropicVersion: string;
  /** 默认最大输出 Token 数。 */
  readonly defaultMaxTokens: number;
  /** 默认思考预算 Token 数。 */
  readonly defaultThinkingBudgetTokens: number;
  /** Messages API 完整 URL。 */
  readonly messagesUrl: string;
}

/**
 * 校验并解析适配器选项。
 *
 * @param options - 用户传入的选项
 * @returns 冻结的解析结果
 * @throws {AnthropicProviderError} API Key、URL、ID、headers、fetch、版本、默认 Token 或思考预算非法
 *
 * @example
 * ```ts
 * const settings = resolveAnthropicProviderOptions({
 *   apiKey: process.env.ANTHROPIC_API_KEY!,
 *   baseUrl: 'https://api.deepseek.com/anthropic',
 * });
 * // settings.messagesUrl === 'https://api.deepseek.com/anthropic/v1/messages'
 * ```
 */
export function resolveAnthropicProviderOptions(
  options: AnthropicProviderOptions,
): ResolvedAnthropicProviderOptions {
  if (typeof options !== 'object' || options === null) {
    throw new AnthropicProviderError(DEFAULT_ANTHROPIC_PROVIDER_ID, '选项必须是对象');
  }
  const apiKey = requireNonEmptyString(options.apiKey, 'apiKey');
  const id =
    options.id === undefined
      ? DEFAULT_ANTHROPIC_PROVIDER_ID
      : requireNonEmptyString(options.id, 'id');
  const baseUrl = resolveBaseUrl(
    id,
    options.baseUrl === undefined
      ? DEFAULT_ANTHROPIC_BASE_URL
      : requireNonEmptyString(options.baseUrl, 'baseUrl'),
  );
  const headers = resolveHeaders(options.headers);
  const fetchImpl = options.fetch ?? globalThis.fetch;
  if (typeof fetchImpl !== 'function') {
    throw new AnthropicProviderError(id, 'fetch 必须是函数');
  }
  const anthropicVersion =
    options.anthropicVersion === undefined
      ? DEFAULT_ANTHROPIC_VERSION
      : requireNonEmptyString(options.anthropicVersion, 'anthropicVersion');
  const defaultMaxTokens = options.defaultMaxTokens ?? DEFAULT_ANTHROPIC_MAX_TOKENS;
  if (!Number.isSafeInteger(defaultMaxTokens) || defaultMaxTokens <= 0) {
    throw new AnthropicProviderError(id, 'defaultMaxTokens 必须是正安全整数');
  }
  const defaultThinkingBudgetTokens =
    options.defaultThinkingBudgetTokens ?? DEFAULT_ANTHROPIC_THINKING_BUDGET_TOKENS;
  if (!Number.isSafeInteger(defaultThinkingBudgetTokens) || defaultThinkingBudgetTokens <= 0) {
    throw new AnthropicProviderError(id, 'defaultThinkingBudgetTokens 必须是正安全整数');
  }

  return Object.freeze({
    apiKey,
    baseUrl,
    id,
    headers,
    fetchImpl,
    anthropicVersion,
    defaultMaxTokens,
    defaultThinkingBudgetTokens,
    messagesUrl: `${baseUrl}/v1/messages`,
  });
}

/** 校验并规范化 API 根地址；拒绝非 HTTP(S) URL 与查询/片段。 */
function resolveBaseUrl(id: string, value: string): string {
  const normalized = value.replace(/\/+$/, '');
  let parsed: URL;
  try {
    parsed = new URL(normalized);
  } catch (cause) {
    throw new AnthropicProviderError(id, 'baseUrl 必须是合法 URL', { cause });
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
    throw new AnthropicProviderError(id, 'baseUrl 只支持 http/https');
  }
  if (parsed.search !== '' || parsed.hash !== '') {
    throw new AnthropicProviderError(id, 'baseUrl 不能包含查询参数或 hash');
  }
  return normalized;
}

/** 校验非空字符串。 */
function requireNonEmptyString(value: unknown, name: string): string {
  if (typeof value !== 'string' || value.trim() === '') {
    throw new AnthropicProviderError(DEFAULT_ANTHROPIC_PROVIDER_ID, `${name} 必须是非空字符串`);
  }
  return value;
}

/** 校验并冻结自定义请求头。 */
function resolveHeaders(value: unknown): Readonly<Record<string, string>> {
  if (value === undefined) return Object.freeze({});
  if (!isPlainObject(value)) {
    throw new AnthropicProviderError(DEFAULT_ANTHROPIC_PROVIDER_ID, 'headers 必须是普通对象');
  }
  const result: Record<string, string> = {};
  for (const [key, headerValue] of Object.entries(value)) {
    if (typeof headerValue !== 'string') {
      throw new AnthropicProviderError(
        DEFAULT_ANTHROPIC_PROVIDER_ID,
        `headers["${key}"] 必须是字符串`,
      );
    }
    result[key] = headerValue;
  }
  return Object.freeze(result);
}
