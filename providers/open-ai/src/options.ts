/**
 * @file 适配器选项解析与校验。
 * @description 所有选项在构造期一次性校验并冻结，运行期不再读取调用方可变对象。
 * @module open-ai/options
 */

import { isPlainObject } from '@kurumi/core';

import { OpenAIProviderError } from './errors.js';

/** 默认 OpenAI API 根地址。 */
export const DEFAULT_OPENAI_BASE_URL = 'https://api.openai.com/v1';

/** 默认提供方 ID。 */
export const DEFAULT_OPENAI_PROVIDER_ID = 'open-ai';

/** 默认推理内容字段名（DeepSeek 等兼容实现使用）。 */
export const DEFAULT_REASONING_CONTENT_FIELD = 'reasoning_content';

/** 默认思考开关字段名（DeepSeek 等兼容实现使用）。 */
export const DEFAULT_THINKING_FIELD = 'thinking';

/** `maxOutputTokens` 使用的请求字段。 */
export type MaxTokensField = 'max_tokens' | 'max_completion_tokens';

/** 思考开关字段不得占用的核心字段。 */
const RESERVED_THINKING_FIELDS: ReadonlySet<string> = new Set([
  'model',
  'messages',
  'stream',
  'stream_options',
  'tools',
  'tool_choice',
  'temperature',
  'max_tokens',
  'max_completion_tokens',
  'reasoning_effort',
]);

/** OpenAI 协议适配器选项。 */
export interface OpenAIProviderOptions {
  /** API Key，必填。 */
  readonly apiKey: string;
  /** API 根地址；缺省 `https://api.openai.com/v1`。 */
  readonly baseUrl?: string;
  /** 提供方 ID，缺省 `open-ai`。 */
  readonly id?: string;
  /** 追加请求头；不能覆盖 content-type 与 authorization。 */
  readonly headers?: Readonly<Record<string, string>>;
  /** 可注入 fetch，便于测试或代理。 */
  readonly fetch?: typeof fetch;
  /** `maxOutputTokens` 映射字段，缺省 `max_tokens`。 */
  readonly maxTokensField?: MaxTokensField;
  /** 流式请求是否发送 `stream_options.include_usage`，缺省 true。 */
  readonly shouldIncludeStreamUsage?: boolean;
  /** 读取推理增量的字段名，缺省 `reasoning_content`。 */
  readonly reasoningContentField?: string;
  /** 思考开关字段名，缺省 `thinking`；设为 false 时不发送开关字段（OpenAI 官方仅使用 reasoning_effort）。 */
  readonly thinkingField?: string | false;
}

/** 解析后的适配器选项。 */
export interface ResolvedOpenAIProviderOptions {
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
  /** 最大输出 Token 字段。 */
  readonly maxTokensField: MaxTokensField;
  /** 是否请求流式 usage。 */
  readonly shouldIncludeStreamUsage: boolean;
  /** 推理内容字段名。 */
  readonly reasoningContentField: string;
  /** 思考开关字段名；false 表示不发送。 */
  readonly thinkingField: string | false;
  /** Chat Completions 完整 URL。 */
  readonly chatUrl: string;
}

/**
 * 校验并解析适配器选项。
 *
 * @param options - 用户传入的选项
 * @returns 冻结的解析结果
 * @throws {OpenAIProviderError} API Key、URL、ID、headers、fetch、枚举字段或思考开关字段非法
 */
export function resolveOpenAIProviderOptions(
  options: OpenAIProviderOptions,
): ResolvedOpenAIProviderOptions {
  if (typeof options !== 'object' || options === null) {
    throw new OpenAIProviderError(DEFAULT_OPENAI_PROVIDER_ID, '选项必须是对象');
  }
  const apiKey = requireNonEmptyString(options.apiKey, 'apiKey');
  const id =
    options.id === undefined ? DEFAULT_OPENAI_PROVIDER_ID : requireNonEmptyString(options.id, 'id');
  const baseUrl = resolveBaseUrl(
    id,
    options.baseUrl === undefined
      ? DEFAULT_OPENAI_BASE_URL
      : requireNonEmptyString(options.baseUrl, 'baseUrl'),
  );
  const headers = resolveHeaders(options.headers);
  const fetchImpl = options.fetch ?? globalThis.fetch;
  if (typeof fetchImpl !== 'function') {
    throw new OpenAIProviderError(id, 'fetch 必须是函数');
  }
  const maxTokensField = options.maxTokensField ?? 'max_tokens';
  if (maxTokensField !== 'max_tokens' && maxTokensField !== 'max_completion_tokens') {
    throw new OpenAIProviderError(id, 'maxTokensField 必须是 max_tokens 或 max_completion_tokens');
  }
  const shouldIncludeStreamUsage = options.shouldIncludeStreamUsage ?? true;
  if (typeof shouldIncludeStreamUsage !== 'boolean') {
    throw new OpenAIProviderError(id, 'shouldIncludeStreamUsage 必须是布尔值');
  }
  const reasoningContentField =
    options.reasoningContentField === undefined
      ? DEFAULT_REASONING_CONTENT_FIELD
      : requireNonEmptyString(options.reasoningContentField, 'reasoningContentField');
  const thinkingField =
    options.thinkingField === undefined
      ? DEFAULT_THINKING_FIELD
      : resolveThinkingField(options.thinkingField);

  return Object.freeze({
    apiKey,
    baseUrl,
    id,
    headers,
    fetchImpl,
    maxTokensField,
    shouldIncludeStreamUsage,
    reasoningContentField,
    thinkingField,
    chatUrl: `${baseUrl}/chat/completions`,
  });
}

/** 校验并规范化 API 根地址；拒绝非 HTTP(S) URL 与查询/片段。 */
function resolveBaseUrl(id: string, value: string): string {
  const normalized = value.replace(/\/+$/, '');
  let parsed: URL;
  try {
    parsed = new URL(normalized);
  } catch (cause) {
    throw new OpenAIProviderError(id, 'baseUrl 必须是合法 URL', { cause });
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
    throw new OpenAIProviderError(id, 'baseUrl 只支持 http/https');
  }
  if (parsed.search !== '' || parsed.hash !== '') {
    throw new OpenAIProviderError(id, 'baseUrl 不能包含查询参数或 hash');
  }
  return normalized;
}

/** 校验非空字符串。 */
function requireNonEmptyString(value: unknown, name: string): string {
  if (typeof value !== 'string' || value.trim() === '') {
    throw new OpenAIProviderError(DEFAULT_OPENAI_PROVIDER_ID, `${name} 必须是非空字符串`);
  }
  return value;
}

/** 校验思考开关字段名；false 表示关闭。 */
function resolveThinkingField(value: unknown): string | false {
  if (value === false) return false;
  if (typeof value !== 'string' || value.trim() === '') {
    throw new OpenAIProviderError(
      DEFAULT_OPENAI_PROVIDER_ID,
      'thinkingField 必须是 false 或非空字符串',
    );
  }
  if (RESERVED_THINKING_FIELDS.has(value)) {
    throw new OpenAIProviderError(
      DEFAULT_OPENAI_PROVIDER_ID,
      `thinkingField 不能使用保留字段 "${value}"`,
    );
  }
  return value;
}

/** 校验并冻结自定义请求头。 */
function resolveHeaders(value: unknown): Readonly<Record<string, string>> {
  if (value === undefined) return Object.freeze({});
  if (!isPlainObject(value)) {
    throw new OpenAIProviderError(DEFAULT_OPENAI_PROVIDER_ID, 'headers 必须是普通对象');
  }
  const result: Record<string, string> = {};
  for (const [key, headerValue] of Object.entries(value)) {
    if (typeof headerValue !== 'string') {
      throw new OpenAIProviderError(DEFAULT_OPENAI_PROVIDER_ID, `headers["${key}"] 必须是字符串`);
    }
    result[key] = headerValue;
  }
  return Object.freeze(result);
}
