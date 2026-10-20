/**
 * @file 通用 schema 的配置适配层。
 * @description 复用 `@kurumi/schema` 的解析器与 JSON Schema 导出；把定义期/校验期错误转换为配置领域错误。
 * @module config/schema/schema
 */

import {
  Schema as BaseSchema,
  SchemaDefinitionError,
  SchemaKind as BaseSchemaKind,
  type Infer,
  type JsonSchema,
  type ParseOutcome as BaseParseOutcome,
  type ParseSuccess,
  type SchemaIssue,
  type SchemaKind as SchemaKindType,
} from '@kurumi/schema';

import { ConfigSchemaDefinitionError, ConfigValidationError } from '../errors.js';
import { createIssue, type ConfigIssue, type ConfigIssueCode } from '../issue.js';

export { isSchema, parseSuccess, toJsonSchema } from '@kurumi/schema';
export type { JsonSchema, ParseSuccess, SchemaMetadata } from '@kurumi/schema';

/** 通用 schema 基类值；用于 `instanceof` 与继承。 */
export const Schema = BaseSchema;

/** 配置 schema 类型别名；函数返回该类型，解析结果使用 `ConfigIssue`。 */
export type Schema<TValue> = ConfigSchema<TValue>;

/** schema 种类值（与通用 schema 包保持一致）。 */
export const SchemaKind = BaseSchemaKind;

/** schema 种类联合类型。 */
export type SchemaKind = SchemaKindType;

/** 解析失败，包含全部配置问题。 */
export interface ParseFailure {
  readonly ok: false;
  readonly issues: readonly ConfigIssue[];
}

/** 配置 schema 解析结果。 */
export type ParseOutcome<TValue> = ParseSuccess<TValue> | ParseFailure;

/** 从 `Schema<TValue>` 推导输出类型。 */
export type { Infer };

/**
 * 构造失败结果。
 *
 * @param issues - 至少一条配置问题
 * @returns 失败结果
 * @throws {ConfigSchemaDefinitionError} issues 为空
 */
export function parseFailure(issues: readonly ConfigIssue[]): ParseFailure {
  if (!Array.isArray(issues) || issues.length === 0) {
    throw new ConfigSchemaDefinitionError('parseFailure 至少需要一条问题');
  }
  return { ok: false, issues: Object.freeze([...issues]) };
}

/**
 * 配置 schema 接口：保留通用 schema 能力，并把解析结果/错误类型固定为配置领域类型。
 */
export interface ConfigSchema<TValue> extends BaseSchema<TValue> {
  parse(value: unknown): ParseOutcome<TValue>;
  parseOrThrow(value: unknown): TValue;
  optional(): ConfigSchema<TValue | undefined>;
  nullable(): ConfigSchema<TValue | null>;
  default(value: TValue): ConfigSchema<TValue>;
  secret(): ConfigSchema<TValue>;
  describe(description: string): ConfigSchema<TValue>;
}

/**
 * 包装通用 schema，使链式 API 与解析错误保持配置领域语义。
 *
 * @param inner - 通用 schema 实例
 * @returns 配置适配后的 schema；已适配时原样返回
 */
export function wrapSchema<TValue>(inner: BaseSchema<TValue>): ConfigSchema<TValue> {
  return inner instanceof ConfigSchemaImpl ? inner : new ConfigSchemaImpl(inner);
}

/**
 * 在定义期捕获通用 schema 错误并转换为 `ConfigSchemaDefinitionError`。
 *
 * @param factory - 构造通用 schema 的工厂
 * @returns 配置适配后的 schema
 * @throws {ConfigSchemaDefinitionError} schema 定义非法
 */
export function defineConfigSchema<TValue>(
  factory: () => BaseSchema<TValue>,
): ConfigSchema<TValue> {
  try {
    return wrapSchema(factory());
  } catch (error) {
    throw toConfigDefinitionError(error);
  }
}

/**
 * 配置 schema 适配器：解析行为委托给通用 schema，错误与链式包装保持配置语义。
 */
class ConfigSchemaImpl<TValue> extends BaseSchema<TValue> implements ConfigSchema<TValue> {
  readonly kind: SchemaKindType;
  readonly #inner: BaseSchema<TValue>;

  constructor(inner: BaseSchema<TValue>) {
    super(inner.metadata);
    this.kind = inner.kind;
    this.#inner = inner;
  }

  _parse(value: unknown, path: string, depth: number): BaseParseOutcome<TValue> {
    return this.#inner._parse(value, path, depth);
  }

  _redact(value: TValue): unknown {
    return this.#inner._redact(value);
  }

  _toJsonSchema(): JsonSchema {
    return this.#inner._toJsonSchema();
  }

  _acceptsUndefined(): boolean {
    return this.#inner._acceptsUndefined();
  }

  override parse(value: unknown): ParseOutcome<TValue> {
    const outcome = this.#inner.parse(value);
    return outcome.ok
      ? outcome
      : { ok: false, issues: Object.freeze(toConfigIssues(outcome.issues)) };
  }

  override parseOrThrow(value: unknown): TValue {
    const outcome = this.#inner.parse(value);
    if (!outcome.ok) throw new ConfigValidationError(toConfigIssues(outcome.issues));
    return outcome.value;
  }

  override optional(): ConfigSchema<TValue | undefined> {
    return wrapSchema(this.#inner.optional());
  }

  override nullable(): ConfigSchema<TValue | null> {
    return wrapSchema(this.#inner.nullable());
  }

  override default(value: TValue): ConfigSchema<TValue> {
    try {
      return wrapSchema(this.#inner.default(value));
    } catch (error) {
      throw toConfigDefinitionError(error);
    }
  }

  override secret(): ConfigSchema<TValue> {
    return wrapSchema(this.#inner.secret());
  }

  override describe(description: string): ConfigSchema<TValue> {
    try {
      return wrapSchema(this.#inner.describe(description));
    } catch (error) {
      throw toConfigDefinitionError(error);
    }
  }
}

/** 把通用 schema issue 转换为配置 issue；通用错误码是配置错误码的子集。 */
function toConfigIssues(issues: readonly SchemaIssue[]): ConfigIssue[] {
  return issues.map((issue) =>
    createIssue(issue.code as ConfigIssueCode, issue.path, issue.message, {
      ...(issue.expected === undefined ? {} : { expected: issue.expected }),
      ...(issue.received === undefined ? {} : { received: issue.received }),
    }),
  );
}

/** 转换定义期错误。 */
function toConfigDefinitionError(error: unknown): unknown {
  if (error instanceof SchemaDefinitionError) {
    return new ConfigSchemaDefinitionError(error.message, error);
  }
  return error;
}
