/**
 * @file schema 基类、元数据、包装器与 JSON Schema 导出。
 * @description 所有 schema 都是不可变对象，解析只返回结果而不修改输入。
 * @module schema/schema
 */

import { cloneJsonValue, deepFreeze, type JsonValue } from '@kurumi/core';

import { SchemaDefinitionError, SchemaValidationError } from './errors.js';
import {
  REDACTED_VALUE,
  ROOT_PATH,
  SchemaIssueCode,
  formatIssues,
  type SchemaIssue,
} from './issue.js';
import type { JsonSchema } from './json-schema.js';

/** schema 种类，供内省、诊断与 JSON Schema 导出使用。 */
export const SchemaKind = Object.freeze({
  String: 'string',
  Number: 'number',
  Boolean: 'boolean',
  Literal: 'literal',
  Array: 'array',
  Object: 'object',
  Record: 'record',
  Union: 'union',
  Optional: 'optional',
  Nullable: 'nullable',
  Default: 'default',
} as const);

/** schema 种类联合类型。 */
export type SchemaKind = (typeof SchemaKind)[keyof typeof SchemaKind];

/** schema 元数据。 */
export interface SchemaMetadata {
  /** 面向文档的字段说明。 */
  readonly description?: string;
  /** 是否为敏感值；为 true 时 redact 会把整个子树替换为占位符。 */
  readonly isSecret: boolean;
}

/** 解析成功。 */
export interface ParseSuccess<TValue> {
  readonly ok: true;
  readonly value: TValue;
}

/** 解析失败，包含全部可聚合的问题。 */
export interface ParseFailure {
  readonly ok: false;
  readonly issues: readonly SchemaIssue[];
}

/** schema 解析结果。 */
export type ParseOutcome<TValue> = ParseSuccess<TValue> | ParseFailure;

/**
 * 构造成功结果。
 *
 * @param value - 已通过 schema 校验的值
 * @returns 成功结果
 */
export function parseSuccess<TValue>(value: TValue): ParseSuccess<TValue> {
  return { ok: true, value };
}

/**
 * 构造失败结果。
 *
 * @remarks 失败分支不含类型信息，可赋给任意 `ParseOutcome<TValue>`。
 * @param issues - 至少一条 schema 问题
 * @returns 失败结果
 * @throws {SchemaDefinitionError} issues 为空
 */
export function parseFailure(issues: readonly SchemaIssue[]): ParseFailure {
  if (!Array.isArray(issues) || issues.length === 0) {
    throw new SchemaDefinitionError('parseFailure 至少需要一条问题');
  }
  return { ok: false, issues: Object.freeze([...issues]) };
}

/**
 * 从 `Schema<TValue>` 推导输出类型。
 *
 * @example
 * ```ts
 * const schema = object({ port: number() });
 * type Config = Infer<typeof schema>; // { readonly port: number }
 * ```
 */
export type Infer<TSchema extends Schema<unknown>> =
  TSchema extends Schema<infer TValue> ? TValue : never;

/**
 * schema 抽象基类。
 *
 * @remarks
 * - 解析不修改输入；成功值由各 schema 自行构造。
 * - `toJsonSchema()` 返回 2020-12 风格的 JSON Schema 子集，供工具定义等边界使用。
 */
export abstract class Schema<TValue> {
  /** 不可变元数据。 */
  readonly metadata: SchemaMetadata;

  /** schema 种类。 */
  abstract readonly kind: SchemaKind;

  /**
   * @param metadata - 已冻结的元数据
   */
  protected constructor(metadata: SchemaMetadata) {
    this.metadata = Object.freeze(metadata);
  }

  /** @internal 按指定路径解析值；供组合 schema 聚合错误。 */
  abstract _parse(value: unknown, path: string, depth: number): ParseOutcome<TValue>;

  /** @internal 按 schema 结构遮蔽敏感值。 */
  abstract _redact(value: TValue): unknown;

  /** @internal 导出原始 JSON Schema；调用方应使用 `toJsonSchema()`。 */
  abstract _toJsonSchema(): JsonSchema;

  /** @internal 判断值缺失（undefined）时是否仍能解析成功。 */
  abstract _acceptsUndefined(): boolean;

  /**
   * 解析任意输入。
   *
   * @param value - 未经信任的输入
   * @returns 成功时包含类型安全的值，失败时包含全部问题
   */
  parse(value: unknown): ParseOutcome<TValue> {
    return this._parse(value, ROOT_PATH, 0);
  }

  /**
   * 解析任意输入，失败时抛出聚合错误。
   *
   * @param value - 未经信任的输入
   * @returns 类型安全的值
   * @throws {SchemaValidationError} 输入不满足 schema
   */
  parseOrThrow(value: unknown): TValue {
    const result = this._parse(value, ROOT_PATH, 0);
    if (!result.ok) throw new SchemaValidationError(result.issues);
    return result.value;
  }

  /**
   * 允许字段缺失（值为 `undefined`）。
   *
   * @returns 新 schema，输出类型追加 `undefined`
   */
  optional(): Schema<TValue | undefined> {
    return new OptionalSchema(this);
  }

  /**
   * 允许显式 `null`。
   *
   * @returns 新 schema，输出类型追加 `null`
   */
  nullable(): Schema<TValue | null> {
    return new NullableSchema(this);
  }

  /**
   * 缺失时使用默认值；默认值会在构造时立即校验并克隆。
   *
   * @param value - 非 undefined 的默认值，必须满足当前 schema 且为 JSON 安全值
   * @returns 新 schema
   * @throws {SchemaDefinitionError} 默认值不满足当前 schema 或不是 JSON 安全值
   */
  default(value: TValue): Schema<TValue> {
    return new DefaultSchema(this, value);
  }

  /**
   * 标记为敏感值。
   *
   * @returns 新 schema；`redact()` 会遮蔽整个子树
   */
  secret(): Schema<TValue> {
    return new MetadataSchema(this, { ...this.metadata, isSecret: true });
  }

  /**
   * 添加字段说明。
   *
   * @param description - 非空说明文本
   * @returns 新 schema
   * @throws {SchemaDefinitionError} 说明为空
   */
  describe(description: string): Schema<TValue> {
    if (typeof description !== 'string' || description.trim() === '') {
      throw new SchemaDefinitionError(
        `schema 描述必须是非空字符串（实际值: ${String(description)}）`,
      );
    }
    return new MetadataSchema(this, { ...this.metadata, description });
  }

  /**
   * 生成可安全展示或持久化的副本。
   *
   * @param value - 已通过本 schema 校验的值
   * @returns 敏感字段被替换为 `[REDACTED]` 的新结构
   */
  redact(value: TValue): unknown {
    return this._redact(value);
  }

  /**
   * 导出 JSON Schema（冻结、JSON 安全）。
   *
   * @remarks
   * - `optional()` / `default()` 在对象中会从 `required` 排除。
   * - `shouldTrim`、字符串强转等解析期行为无法用 JSON Schema 表达，仅作为服务端校验语义。
   * @returns 冻结的 JSON Schema 对象
   */
  toJsonSchema(): JsonSchema {
    const base = this._toJsonSchema();
    const described =
      this.metadata.description === undefined
        ? base
        : { ...base, description: this.metadata.description };
    return deepFreeze(described);
  }
}

/**
 * 判断任意值是否为 schema 实例。
 *
 * @param value - 待判断值
 * @returns 是 `Schema` 实例时返回 true
 */
export function isSchema(value: unknown): value is Schema<unknown> {
  return value instanceof Schema;
}

/**
 * 导出任意 schema 的 JSON Schema。
 *
 * @param schema - 目标 schema
 * @returns 冻结的 JSON Schema 对象
 */
export function toJsonSchema<TValue>(schema: Schema<TValue>): JsonSchema {
  return schema.toJsonSchema();
}

/** 可选包装器。 */
class OptionalSchema<TValue> extends Schema<TValue | undefined> {
  readonly kind = SchemaKind.Optional;
  readonly #inner: Schema<TValue>;

  constructor(inner: Schema<TValue>) {
    super(inner.metadata);
    this.#inner = inner;
  }

  _parse(value: unknown, path: string, depth: number): ParseOutcome<TValue | undefined> {
    return value === undefined ? parseSuccess(undefined) : this.#inner._parse(value, path, depth);
  }

  _redact(value: TValue | undefined): unknown {
    return value === undefined ? undefined : this.#inner._redact(value);
  }

  _toJsonSchema(): JsonSchema {
    return this.#inner._toJsonSchema();
  }

  _acceptsUndefined(): boolean {
    return true;
  }
}

/** 可空包装器。 */
class NullableSchema<TValue> extends Schema<TValue | null> {
  readonly kind = SchemaKind.Nullable;
  readonly #inner: Schema<TValue>;

  constructor(inner: Schema<TValue>) {
    super(inner.metadata);
    this.#inner = inner;
  }

  _parse(value: unknown, path: string, depth: number): ParseOutcome<TValue | null> {
    return value === null ? parseSuccess(null) : this.#inner._parse(value, path, depth);
  }

  _redact(value: TValue | null): unknown {
    return value === null ? null : this.#inner._redact(value);
  }

  _toJsonSchema(): JsonSchema {
    return { anyOf: [this.#inner._toJsonSchema(), { type: 'null' }] };
  }

  _acceptsUndefined(): boolean {
    // nullable 本身不额外接受 undefined，但内层 optional/default 可能接受；必须委托，
    // 否则 toJsonSchema() 会把“缺失时可用默认值/可选”的字段错误标记为 required。
    return this.#inner._acceptsUndefined();
  }
}

/** 默认值包装器。 */
class DefaultSchema<TValue> extends Schema<TValue> {
  readonly kind = SchemaKind.Default;
  readonly #inner: Schema<TValue>;
  readonly #defaultValue: JsonValue;

  constructor(inner: Schema<TValue>, defaultValue: TValue) {
    super(inner.metadata);
    if (defaultValue === undefined) {
      throw new SchemaDefinitionError('default() 不接受 undefined；请改用 optional()');
    }
    const validation = inner._parse(defaultValue, ROOT_PATH, 0);
    if (!validation.ok) {
      throw new SchemaDefinitionError(`默认值不满足 schema:\n${formatIssues(validation.issues)}`);
    }
    this.#inner = inner;
    // 克隆默认值，避免调用方后续修改默认对象，也避免冻结配置时污染 schema 自身。
    try {
      this.#defaultValue = cloneJsonValue(defaultValue, '$.schema.default');
    } catch (error) {
      throw new SchemaDefinitionError('默认值必须是 JSON 安全值', error);
    }
  }

  _parse(value: unknown, path: string, depth: number): ParseOutcome<TValue> {
    return this.#inner._parse(value === undefined ? this.#defaultValue : value, path, depth);
  }

  _redact(value: TValue): unknown {
    return this.#inner._redact(value);
  }

  _toJsonSchema(): JsonSchema {
    return { ...this.#inner._toJsonSchema(), default: this.#defaultValue };
  }

  _acceptsUndefined(): boolean {
    return true;
  }
}

/** 元数据包装器：不改解析行为，只改描述或敏感标记。 */
class MetadataSchema<TValue> extends Schema<TValue> {
  readonly kind: SchemaKind;
  readonly #inner: Schema<TValue>;

  constructor(inner: Schema<TValue>, metadata: SchemaMetadata) {
    super(metadata);
    this.kind = inner.kind;
    this.#inner = inner;
  }

  _parse(value: unknown, path: string, depth: number): ParseOutcome<TValue> {
    const outcome = this.#inner._parse(value, path, depth);
    if (!this.metadata.isSecret || outcome.ok) return outcome;
    // secret 校验失败时只保留路径与期望值，避免把实际敏感值写入错误消息。
    return parseFailure(
      outcome.issues.map((issue) =>
        issue.code === SchemaIssueCode.Required
          ? Object.freeze({
              ...issue,
              // 必填缺失没有实际值可泄漏，保留原因，避免用户误以为是值格式错误。
              message: '缺少必填的敏感字段（实际值: undefined）',
              received: 'undefined',
            })
          : Object.freeze({
              ...issue,
              message: '敏感字段校验失败',
              received: REDACTED_VALUE,
            }),
      ),
    );
  }

  _redact(value: TValue): unknown {
    return this.metadata.isSecret ? REDACTED_VALUE : this.#inner._redact(value);
  }

  _toJsonSchema(): JsonSchema {
    return this.#inner._toJsonSchema();
  }

  _acceptsUndefined(): boolean {
    return this.#inner._acceptsUndefined();
  }
}
