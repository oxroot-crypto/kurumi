/**
 * @file Anthropic 协议适配器错误。
 * @description 保留 providerId、HTTP 状态与 cause；不得包含 x-api-key 头或 API Key。
 * @module anthropic/errors
 */

import { KurumiError } from '@kurumi/core';

/** Anthropic 协议适配器错误选项。 */
export interface AnthropicProviderErrorOptions {
  /** HTTP 状态码；非 HTTP 错误时缺省。 */
  readonly status?: number;
  /** 底层错误。 */
  readonly cause?: unknown;
}

/** Anthropic 协议适配器错误。 */
export class AnthropicProviderError extends KurumiError {
  /** 提供方 ID。 */
  readonly providerId: string;
  /** HTTP 状态码；非 HTTP 错误时为 undefined。 */
  readonly status: number | undefined;

  /**
   * @param providerId - 提供方 ID
   * @param message - 失败描述（不得包含敏感信息）
   * @param options - HTTP 状态与 cause
   */
  constructor(providerId: string, message: string, options?: AnthropicProviderErrorOptions) {
    super(`Anthropic 提供方 "${providerId}" ${message}`, {
      code: 'anthropic_provider_error',
      ...(options?.cause === undefined ? {} : { cause: options.cause }),
    });
    this.providerId = providerId;
    this.status = options?.status;
  }
}

/**
 * 从诊断文本中移除 API Key。
 *
 * @param value - 原始诊断文本
 * @param secret - 需要遮蔽的 API Key
 * @returns 将 secret 替换为 `[REDACTED]` 的文本
 */
export function redactAnthropicSecret(value: string, secret: string): string {
  if (secret === '') return value;
  return value.split(secret).join('[REDACTED]');
}
