/**
 * @file 通用 schema 层错误基类。
 * @description 复用 core 的 KurumiError（code/cause/name），并区分定义期与校验期失败。
 * @module schema/errors
 */

import { KurumiError, type KurumiErrorOptions } from '@kurumi/core';

import { formatIssues, type SchemaIssue } from './issue.js';

/** schema 错误构造选项。 */
export type SchemaErrorOptions = KurumiErrorOptions;

/** schema 包所有错误的基类。 */
export class SchemaError extends KurumiError {
  /**
   * @param message - 面向人类的错误描述
   * @param options - 错误码与 cause；错误码缺省为 `schema_error`
   */
  constructor(message: string, options?: SchemaErrorOptions) {
    super(message, {
      ...(options?.code === undefined ? { code: 'schema_error' } : { code: options.code }),
      ...(options?.cause === undefined ? {} : { cause: options.cause }),
    });
  }
}

/** schema 定义本身非法（例如负数长度上限、非法默认值）。 */
export class SchemaDefinitionError extends SchemaError {
  /**
   * @param message - 描述哪条 schema 约束非法
   * @param cause - 底层定义期错误
   */
  constructor(message: string, cause?: unknown) {
    super(message, {
      code: 'schema_definition_error',
      ...(cause === undefined ? {} : { cause }),
    });
  }
}

/** schema 校验失败：聚合所有字段级问题。 */
export class SchemaValidationError extends SchemaError {
  /** 全部校验问题，按发现顺序排列。 */
  readonly issues: readonly SchemaIssue[];

  /**
   * @param issues - 至少一条校验问题
   * @throws {SchemaDefinitionError} issues 为空
   */
  constructor(issues: readonly SchemaIssue[]) {
    if (!Array.isArray(issues) || issues.length === 0) {
      throw new SchemaDefinitionError('SchemaValidationError 至少需要一条问题');
    }
    super(`schema 校验失败（${issues.length} 个问题）:\n${formatIssues(issues)}`, {
      code: 'schema_validation_error',
    });
    this.issues = Object.freeze([...issues]);
  }
}
