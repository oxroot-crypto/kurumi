/**
 * @file 配置用标量 schema 包装。
 * @description 在通用 schema 基础上启用环境变量字符串强转，保持配置系统的既有语义。
 * @module config/schema/scalars
 */

import { isPlainObject } from '@kurumi/core';
import {
  boolean as schemaBoolean,
  literal as schemaLiteral,
  number as schemaNumber,
  oneOf as schemaOneOf,
  string as schemaString,
  type BooleanSchemaOptions,
  type LiteralOptions,
  type LiteralValue,
  type NumberSchemaOptions,
  type StringSchemaOptions,
} from '@kurumi/schema';

import { ConfigSchemaDefinitionError } from '../errors.js';
import { defineConfigSchema, type ConfigSchema } from './schema.js';

/** 配置 schema 选项必须是普通对象。 */
function assertOptions(name: string, value: unknown): void {
  if (!isPlainObject(value)) {
    throw new ConfigSchemaDefinitionError(`${name} 必须是普通对象（实际值: ${String(value)}）`);
  }
}

export type {
  BooleanSchemaOptions,
  LiteralOptions,
  LiteralValue,
  NumberSchemaOptions,
  StringSchemaOptions,
};

/**
 * 创建字符串 schema。
 *
 * @param options - 长度、正则、trim、非空约束
 * @returns 字符串 schema
 * @throws {ConfigSchemaDefinitionError} 选项非法
 */
export function string(options: StringSchemaOptions = {}): ConfigSchema<string> {
  assertOptions('string options', options);
  return defineConfigSchema(() => schemaString(options));
}

/**
 * 创建数值 schema；默认接受严格数字字符串（环境变量需要）。
 *
 * @param options - 范围、整数、非有限值及字符串强转策略
 * @returns 数值 schema
 * @throws {ConfigSchemaDefinitionError} 选项非法
 */
export function number(options: NumberSchemaOptions = {}): ConfigSchema<number> {
  assertOptions('number options', options);
  return defineConfigSchema(() =>
    schemaNumber(
      options.shouldCoerceStrings === undefined
        ? { ...options, shouldCoerceStrings: true }
        : options,
    ),
  );
}

/**
 * 创建布尔 schema；默认接受 `true/false/1/0` 字符串（环境变量需要）。
 *
 * @param options - 字符串强转策略
 * @returns 布尔 schema
 * @throws {ConfigSchemaDefinitionError} 选项非法
 */
export function boolean(options: BooleanSchemaOptions = {}): ConfigSchema<boolean> {
  assertOptions('boolean options', options);
  return defineConfigSchema(() =>
    schemaBoolean(
      options.shouldCoerceStrings === undefined
        ? { ...options, shouldCoerceStrings: true }
        : options,
    ),
  );
}

/**
 * 创建单个字面量 schema；默认接受数字/布尔字符串形式。
 *
 * @param expected - 期望的字面量
 * @param options - 字符串强转策略
 * @returns 字面量 schema
 * @throws {ConfigSchemaDefinitionError} 期望值或选项非法
 */
export function literal<TValue extends LiteralValue>(
  expected: TValue,
  options: LiteralOptions = {},
): ConfigSchema<TValue> {
  assertOptions('literal options', options);
  return defineConfigSchema(() =>
    schemaLiteral(
      expected,
      options.shouldCoerceStrings === undefined
        ? { ...options, shouldCoerceStrings: true }
        : options,
    ),
  );
}

/**
 * 创建字面量集合 schema；默认接受数字/布尔字符串形式。
 *
 * @param values - 至少一个合法字面量
 * @param options - 字符串强转策略
 * @returns 字面量联合 schema
 * @throws {ConfigSchemaDefinitionError} 集合为空或包含非法字面量
 */
export function oneOf<const TValues extends readonly LiteralValue[]>(
  values: TValues,
  options: LiteralOptions = {},
): ConfigSchema<TValues[number]> {
  assertOptions('oneOf options', options);
  return defineConfigSchema(() =>
    schemaOneOf(
      values,
      options.shouldCoerceStrings === undefined
        ? { ...options, shouldCoerceStrings: true }
        : options,
    ),
  );
}
