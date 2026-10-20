/**
 * @file 组合 schema 公共工厂、选项与输出类型。
 * @description 负责字段、未知键、长度边界、危险键与深度防护；默认按严格 JSON 语义解析。
 * @module schema/composites
 */

import type { JsonValue } from '@kurumi/core';

import { ArraySchema, RecordSchema, UnionSchema } from './composite-schemas.js';
import { SchemaDefinitionError } from './errors.js';
import { ObjectSchema } from './object-schema.js';
import { Schema, type Infer } from './schema.js';
import {
  assertOptionalString,
  assertOptionsObject,
  assertSchema,
  assertSchemaShape,
} from './support.js';

/** 对象 schema 的字段表。 */
export type SchemaShape = Readonly<Record<string, Schema<unknown>>>;

/** 对象 schema 输出类型。 */
export type InferShape<TShape extends SchemaShape> = {
  readonly [TKey in keyof TShape]: Infer<TShape[TKey]>;
};

/**
 * 对象 schema 输出类型。
 *
 * @remarks
 * `passthrough` 会在运行时保留未知键，因此类型必须同步暴露这些键，避免“运行时值比类型多”的
 * 不安全状态；其他策略只输出 shape 中声明的字段。
 */
export type InferObject<
  TShape extends SchemaShape,
  TOptions extends ObjectSchemaOptions,
> = TOptions extends { readonly unknownKeys: 'passthrough' }
  ? InferShape<TShape> & Readonly<Record<string, JsonValue>>
  : InferShape<TShape>;

/** 未知键处理策略。 */
export type UnknownKeysPolicy = 'error' | 'strip' | 'passthrough';

/** 对象 schema 选项。 */
export interface ObjectSchemaOptions {
  /**
   * 未知键策略，缺省 `error`。
   * - `error`：报错，适合发现拼写错误；
   * - `strip`：丢弃；
   * - `passthrough`：原样保留（不参与校验）。
   */
  readonly unknownKeys?: UnknownKeysPolicy;
  /**
   * 整个对象 section 缺失时的策略，缺省 `error`。
   * - `empty`：按 `{}` 解析，让嵌套默认值生效；
   * - `error`：按必填字段缺失报错。
   */
  readonly missingSection?: 'empty' | 'error';
}

/** 数组 schema 选项。 */
export interface ArraySchemaOptions {
  /** 最小元素数量。 */
  readonly minLength?: number;
  /** 最大元素数量。 */
  readonly maxLength?: number;
}

/** 记录 schema 选项。 */
export interface RecordSchemaOptions {
  /** 键名正则；内部会移除 g/y 标志。 */
  readonly keyPattern?: RegExp;
  /** 键名正则失败时的自定义消息。 */
  readonly keyPatternMessage?: string;
  /** 最小键数量。 */
  readonly minKeys?: number;
  /** 最大键数量。 */
  readonly maxKeys?: number;
}

/**
 * 创建对象 schema。
 *
 * @param shape - 字段名到 schema 的映射
 * @param options - 未知键与缺失 section 策略
 * @returns 对象 schema
 * @throws {SchemaDefinitionError} 字段名非法或策略非法
 *
 * @example
 * ```ts
 * const server = object({
 *   host: string().default('127.0.0.1'),
 *   port: number({ isInteger: true, min: 1, max: 65_535 }),
 * });
 * ```
 */
export function object<
  const TShape extends SchemaShape,
  const TOptions extends ObjectSchemaOptions = {},
>(shape: TShape, options: TOptions = {} as TOptions): Schema<InferObject<TShape, TOptions>> {
  assertSchemaShape('object shape', shape);
  assertOptionsObject('object options', options);
  return new ObjectSchema<TShape, TOptions>(shape, options);
}

/**
 * 创建数组 schema（输出只读数组）。
 *
 * @param itemSchema - 元素 schema
 * @param options - 长度约束
 * @returns 数组 schema
 * @throws {SchemaDefinitionError} 选项非法
 */
export function array<TItem>(
  itemSchema: Schema<TItem>,
  options: ArraySchemaOptions = {},
): Schema<readonly TItem[]> {
  assertSchema('array itemSchema', itemSchema);
  assertOptionsObject('array options', options);
  return new ArraySchema(itemSchema, options);
}

/**
 * 创建字符串键记录 schema。
 *
 * @param valueSchema - 所有值的 schema
 * @param options - 键正则与数量约束
 * @returns 记录 schema
 * @throws {SchemaDefinitionError} 选项非法
 */
export function record<TValue>(
  valueSchema: Schema<TValue>,
  options: RecordSchemaOptions = {},
): Schema<Readonly<Record<string, TValue>>> {
  assertSchema('record valueSchema', valueSchema);
  assertOptionsObject('record options', options);
  if (options.keyPattern !== undefined && !(options.keyPattern instanceof RegExp)) {
    throw new SchemaDefinitionError('record options.keyPattern 必须是 RegExp');
  }
  assertOptionalString('record options.keyPatternMessage', options.keyPatternMessage);
  return new RecordSchema(valueSchema, options);
}

/**
 * 创建联合 schema（按声明顺序尝试，返回第一个匹配分支）。
 *
 * @param options - 至少两个 schema
 * @returns 输出为各分支输出的联合
 * @throws {SchemaDefinitionError} 分支少于两个
 */
export function union<const TOptions extends readonly Schema<unknown>[]>(
  options: TOptions,
): Schema<Infer<TOptions[number]>> {
  if (!Array.isArray(options)) {
    throw new SchemaDefinitionError('union() 需要 Schema 数组');
  }
  for (const [index, option] of options.entries()) {
    assertSchema(`union options[${String(index)}]`, option);
  }
  return new UnionSchema(options);
}
