/**
 * @file 标量 schema 公共工厂与选项类型。
 * @description 默认按严格 JSON 语义解析；字符串强转必须显式开启（配置系统会开启）。
 * @module schema/scalars
 */

import type { JsonPrimitive } from '@kurumi/core';

import { SchemaDefinitionError } from './errors.js';
import {
  BooleanSchema,
  LiteralSchema,
  LiteralSetSchema,
  NumberSchema,
  StringSchema,
  assertBooleanOption,
  assertValidLiteral,
} from './scalar-schemas.js';
import type { Schema } from './schema.js';
import { assertOptionalString, assertOptionsObject } from './support.js';

export { NUMERIC_STRING_PATTERN } from './scalar-schemas.js';

/** 字面量允许的原始值。 */
export type LiteralValue = JsonPrimitive;

/** 字符串 schema 选项。 */
export interface StringSchemaOptions {
  /** 最小长度（Unicode 码点），必须为非负整数。 */
  readonly minLength?: number;
  /** 最大长度（Unicode 码点），必须为非负整数且不小于 minLength。 */
  readonly maxLength?: number;
  /** 正则约束；内部会移除 g/y 标志避免状态漂移。 */
  readonly pattern?: RegExp;
  /** 正则失败时的自定义消息。 */
  readonly patternMessage?: string;
  /** 校验前是否去除首尾空白，缺省 false。 */
  readonly shouldTrim?: boolean;
  /** 是否要求非空（等价于 trim 后最小长度 1）。 */
  readonly isNonEmpty?: boolean;
}

/** 数值 schema 选项。 */
export interface NumberSchemaOptions {
  /** 下界（含）。 */
  readonly min?: number;
  /** 上界（含）。 */
  readonly max?: number;
  /** 是否要求整数，缺省 false。 */
  readonly isInteger?: boolean;
  /** 是否允许 ±Infinity，缺省 false；NaN 永远拒绝。 */
  readonly allowNonFinite?: boolean;
  /** 是否接受严格数字字符串，缺省 false；配置系统会显式开启。 */
  readonly shouldCoerceStrings?: boolean;
}

/** 布尔 schema 选项。 */
export interface BooleanSchemaOptions {
  /** 是否接受 `true/false/1/0` 字符串（大小写不敏感），缺省 false。 */
  readonly shouldCoerceStrings?: boolean;
}

/** 字面量强转选项。 */
export interface LiteralOptions {
  /** 是否接受数字/布尔的环境变量字符串形式，缺省 false。 */
  readonly shouldCoerceStrings?: boolean;
}

/**
 * 创建字符串 schema。
 *
 * @param options - 长度、正则、trim、非空约束
 * @returns 字符串 schema
 * @throws {SchemaDefinitionError} 选项非法
 *
 * @example
 * ```ts
 * const env = string({ pattern: /^(development|production)$/ });
 * ```
 */
export function string(options: StringSchemaOptions = {}): Schema<string> {
  assertOptionsObject('string options', options);
  if (options.pattern !== undefined && !(options.pattern instanceof RegExp)) {
    throw new SchemaDefinitionError('string options.pattern 必须是 RegExp');
  }
  assertOptionalString('string options.patternMessage', options.patternMessage);
  return new StringSchema(options);
}

/**
 * 创建数值 schema。
 *
 * @param options - 范围、整数、非有限值及字符串强转策略
 * @returns 数值 schema
 * @throws {SchemaDefinitionError} 选项非法
 *
 * @example
 * ```ts
 * number({ isInteger: true, min: 1, max: 65_535 }); // 端口
 * ```
 */
export function number(options: NumberSchemaOptions = {}): Schema<number> {
  assertOptionsObject('number options', options);
  return new NumberSchema(options);
}

/**
 * 创建布尔 schema。
 *
 * @param options - 字符串强转策略
 * @returns 布尔 schema
 * @throws {SchemaDefinitionError} 选项非法
 */
export function boolean(options: BooleanSchemaOptions = {}): Schema<boolean> {
  assertOptionsObject('boolean options', options);
  return new BooleanSchema(options);
}

/**
 * 创建单个字面量 schema。
 *
 * @param expected - 期望的字面量（字符串、有限数值、布尔或 null）
 * @param options - 字符串强转策略
 * @returns 字面量 schema
 * @throws {SchemaDefinitionError} 期望值为 NaN/Infinity 或选项非法
 */
export function literal<TValue extends LiteralValue>(
  expected: TValue,
  options: LiteralOptions = {},
): Schema<TValue> {
  assertOptionsObject('literal options', options);
  assertValidLiteral(expected);
  assertBooleanOption('shouldCoerceStrings', options.shouldCoerceStrings);
  return new LiteralSchema(expected, options.shouldCoerceStrings ?? false);
}

/**
 * 创建字面量集合 schema，等价于多个 literal 的联合。
 *
 * @param values - 至少一个合法字面量
 * @param options - 字符串强转策略
 * @returns 输出为 `values` 元素联合的 schema
 * @throws {SchemaDefinitionError} 集合为空或包含非法字面量
 */
export function oneOf<const TValues extends readonly LiteralValue[]>(
  values: TValues,
  options: LiteralOptions = {},
): Schema<TValues[number]> {
  assertOptionsObject('oneOf options', options);
  if (!Array.isArray(values) || values.length === 0) {
    throw new SchemaDefinitionError('oneOf() 至少需要一个字面量');
  }
  assertBooleanOption('shouldCoerceStrings', options.shouldCoerceStrings);
  for (const value of values) assertValidLiteral(value);
  return new LiteralSetSchema(values, options.shouldCoerceStrings ?? false);
}
