/**
 * @file 日志领域错误。
 * @description 复用 core 的 KurumiError 语义，提供定义期、关闭期与 sink 失败分类。
 * @module log/errors
 */

import { KurumiError, type KurumiErrorOptions } from '@kurumi/core';

/** 日志包所有错误的基类。 */
export class LogError extends KurumiError {
  /**
   * @param message - 面向人类的错误描述
   * @param options - 错误码与 cause
   */
  constructor(message: string, options?: KurumiErrorOptions) {
    super(message, { code: 'log_error', ...options });
  }
}

/** 日志器或 sink 的定义不合法。 */
export class LogDefinitionError extends LogError {
  /**
   * @param message - 非法的选项或结构描述
   */
  constructor(message: string) {
    super(message, { code: 'log_definition_error' });
  }
}

/** sink 写入、刷新或关闭失败。 */
export class LogSinkError extends LogError {
  /** 出错的 sink 名称；匿名 sink 使用 `<anonymous>`。 */
  readonly sinkName: string;

  /**
   * @param sinkName - sink 名称
   * @param message - 失败描述
   * @param cause - 底层异常
   */
  constructor(sinkName: string, message: string, cause?: unknown) {
    super(`日志 sink "${sinkName}" 失败: ${message}`, {
      code: 'log_sink_error',
      ...(cause === undefined ? {} : { cause }),
    });
    this.sinkName = sinkName;
  }
}

/** 日志器已关闭，不能再写入。 */
export class LogClosedError extends LogError {
  /** 创建“日志器已关闭”错误。 */
  constructor() {
    super('日志器已关闭，不能再写入', { code: 'log_closed' });
  }
}
