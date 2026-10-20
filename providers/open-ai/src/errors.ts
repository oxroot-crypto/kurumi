/**
 * @file OpenAI 协议适配器错误。
 * @description 保留 providerId、HTTP 状态与 cause；不得包含 Authorization 头或 API Key。
 * @module open-ai/errors
 */

import { KurumiError } from '@kurumi/core';

/** OpenAI 协议适配器错误选项。 */
export interface OpenAIProviderErrorOptions {
  /** HTTP 状态码；非 HTTP 错误时缺省。 */
  readonly status?: number;
  /** 底层错误。 */
  readonly cause?: unknown;
}

/** OpenAI 协议适配器错误。 */
export class OpenAIProviderError extends KurumiError {
  /** 提供方 ID。 */
  readonly providerId: string;
  /** HTTP 状态码；非 HTTP 错误时为 undefined。 */
  readonly status: number | undefined;

  /**
   * @param providerId - 提供方 ID
   * @param message - 失败描述（不得包含敏感信息）
   * @param options - HTTP 状态与 cause
   */
  constructor(providerId: string, message: string, options?: OpenAIProviderErrorOptions) {
    super(`OpenAI 提供方 "${providerId}" ${message}`, {
      code: 'open_ai_provider_error',
      ...(options?.cause === undefined ? {} : { cause: options.cause }),
    });
    this.providerId = providerId;
    this.status = options?.status;
  }
}
