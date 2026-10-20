/**
 * @file 数据提供方错误体系。
 * @description 区分 HTTP、响应数据与不支持能力；所有错误携带 provider id，便于多数据源排障。
 * @module finance/provider/errors
 */

import type { KurumiErrorOptions } from '@kurumi/core';

import { FinanceError } from '../errors.js';

/** 数据提供方错误基类。 */
export class ProviderError extends FinanceError {
  /** 提供方 id。 */
  readonly providerId: string;

  /**
   * @param providerId - 提供方 id
   * @param message - 失败描述
   * @param options - 错误码与 cause
   */
  constructor(providerId: string, message: string, options?: KurumiErrorOptions) {
    super(`[${providerId}] ${message}`, { code: 'provider_error', ...options });
    this.providerId = providerId;
  }
}

/** HTTP 请求失败。 */
export class ProviderHttpError extends ProviderError {
  /** HTTP 状态码；网络错误时为 0。 */
  readonly status: number;
  /** 请求 URL。 */
  readonly url: string;

  /**
   * @param providerId - 提供方 id
   * @param url - 请求 URL
   * @param status - HTTP 状态码；网络错误为 0
   * @param message - 失败描述
   * @param cause - 底层错误
   */
  constructor(providerId: string, url: string, status: number, message: string, cause?: unknown) {
    super(providerId, `HTTP ${status} ${url}: ${message}`, {
      code: 'provider_http_error',
      ...(cause === undefined ? {} : { cause }),
    });
    this.status = status;
    this.url = url;
  }
}

/** 响应结构或字段不符合预期。 */
export class ProviderDataError extends ProviderError {
  /**
   * @param providerId - 提供方 id
   * @param message - 数据错误描述
   * @param cause - 底层错误
   */
  constructor(providerId: string, message: string, cause?: unknown) {
    super(providerId, `响应数据非法: ${message}`, {
      code: 'provider_data_error',
      ...(cause === undefined ? {} : { cause }),
    });
  }
}

/** 数据源不支持该只读能力。 */
export class UnsupportedProviderOperationError extends ProviderError {
  /** 不支持的方法名。 */
  readonly operation: string;

  /**
   * @param providerId - 提供方 id
   * @param operation - 方法名
   */
  constructor(providerId: string, operation: string) {
    super(providerId, `不支持操作 ${operation}`, {
      code: 'provider_unsupported',
    });
    this.operation = operation;
  }
}
