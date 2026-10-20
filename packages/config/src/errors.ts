/**
 * @file 配置层错误基类。
 * @description 复用 core 的 KurumiError（code/cause/name），并补齐配置领域错误类型。
 * @module config/errors
 */

import { KurumiError, type KurumiErrorOptions } from '@kurumi/core';

import { formatIssues, type ConfigIssue } from './issue.js';

/** 错误构造选项。 */
export type ConfigErrorOptions = KurumiErrorOptions;

/** 配置包所有错误的基类。 */
export class ConfigError extends KurumiError {
  /**
   * @param message - 面向人类的错误描述
   * @param options - 错误码与 cause；错误码缺省为 `config_error`
   */
  constructor(message: string, options?: ConfigErrorOptions) {
    super(message, {
      ...(options?.code === undefined ? { code: 'config_error' } : { code: options.code }),
      ...(options?.cause === undefined ? {} : { cause: options.cause }),
    });
  }
}

/** 配置校验失败：聚合所有字段级问题。 */
export class ConfigValidationError extends ConfigError {
  /** 全部校验问题，按发现顺序排列。 */
  readonly issues: readonly ConfigIssue[];

  /**
   * @param issues - 至少一条校验问题
   */
  constructor(issues: readonly ConfigIssue[]) {
    if (!Array.isArray(issues) || issues.length === 0) {
      throw new ConfigSchemaDefinitionError('ConfigValidationError 至少需要一条问题');
    }
    super(`配置校验失败（${issues.length} 个问题）:\n${formatIssues(issues)}`, {
      code: 'validation_error',
    });
    this.issues = Object.freeze([...issues]);
  }
}

/** 配置来源加载失败（读取文件、解析格式、映射环境变量等）。 */
export class ConfigSourceError extends ConfigError {
  /** 出错的来源名称。 */
  readonly sourceName: string;

  /**
   * @param sourceName - 来源名称
   * @param message - 错误描述，应包含来源定位信息
   * @param options - 错误码与 cause
   */
  constructor(sourceName: string, message: string, options?: ConfigErrorOptions) {
    super(`配置来源 "${sourceName}" 加载失败: ${message}`, {
      ...(options?.code === undefined ? { code: 'source_error' } : { code: options.code }),
      ...(options?.cause === undefined ? {} : { cause: options.cause }),
    });
    this.sourceName = sourceName;
  }
}

/** JSON/JSONC 语法错误，携带清理后文本中的偏移量。 */
export class ConfigSyntaxError extends ConfigError {
  /** 来源名称（通常是文件路径）。 */
  readonly sourceName: string;
  /** 清理注释后文本中的字符偏移，0 起。 */
  readonly offset: number;

  /**
   * @param sourceName - 来源名称
   * @param message - 语法错误描述
   * @param offset - 错误偏移量
   * @param cause - `JSON.parse` 抛出的原始错误
   */
  constructor(sourceName: string, message: string, offset: number, cause?: unknown) {
    super(`配置文件 "${sourceName}" 语法错误（偏移 ${offset}）: ${message}`, {
      code: 'syntax_error',
      ...(cause === undefined ? {} : { cause }),
    });
    this.sourceName = sourceName;
    this.offset = offset;
  }
}

/** 环境变量到配置树的映射失败。 */
export class ConfigEnvMappingError extends ConfigSourceError {
  /** 全部映射问题。 */
  readonly issues: readonly ConfigIssue[];

  /**
   * @param sourceName - 来源名称
   * @param issues - 映射问题列表，至少一条
   */
  constructor(sourceName: string, issues: readonly ConfigIssue[]) {
    if (!Array.isArray(issues) || issues.length === 0) {
      throw new ConfigSchemaDefinitionError('ConfigEnvMappingError 至少需要一条问题');
    }
    super(sourceName, `环境变量映射失败（${issues.length} 个问题）:\n${formatIssues(issues)}`, {
      code: 'env_mapping_error',
    });
    this.issues = Object.freeze([...issues]);
  }
}

/** 字符串插值失败。 */
export class ConfigInterpolationError extends ConfigError {
  /** 全部插值问题。 */
  readonly issues: readonly ConfigIssue[];

  /**
   * @param issues - 至少一条插值问题
   */
  constructor(issues: readonly ConfigIssue[]) {
    if (!Array.isArray(issues) || issues.length === 0) {
      throw new ConfigSchemaDefinitionError('ConfigInterpolationError 至少需要一条问题');
    }
    super(`配置插值失败（${issues.length} 个问题）:\n${formatIssues(issues)}`, {
      code: 'interpolation_error',
    });
    this.issues = Object.freeze([...issues]);
  }
}

/** schema 定义本身非法（例如负数长度上限、非法默认值）。 */
export class ConfigSchemaDefinitionError extends ConfigError {
  /**
   * @param message - 描述哪条 schema 约束非法
   * @param cause - 底层通用 schema 定义错误
   */
  constructor(message: string, cause?: unknown) {
    super(message, {
      code: 'schema_definition_error',
      ...(cause === undefined ? {} : { cause }),
    });
  }
}

/** 对已销毁的配置存储执行操作。 */
export class ConfigDisposedError extends ConfigError {
  /**
   * 创建“存储已销毁”错误。
   */
  constructor() {
    super('配置存储已销毁，不能再 reload/subscribe', { code: 'disposed' });
  }
}
