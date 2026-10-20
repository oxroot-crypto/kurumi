/**
 * @file 配置用组合 schema 包装。
 * @description 复用通用组合 schema；对象 section 缺失时默认按空对象解析，让嵌套默认值生效。
 * @module config/schema/composites
 */

import { isPlainObject } from '@kurumi/core';
import {
  array as schemaArray,
  object as schemaObject,
  record as schemaRecord,
  union as schemaUnion,
  type ArraySchemaOptions,
  type InferObject,
  type InferShape,
  type ObjectSchemaOptions,
  type RecordSchemaOptions,
  type Schema as BaseSchema,
  type SchemaShape,
  type UnknownKeysPolicy,
} from '@kurumi/schema';

import { ConfigSchemaDefinitionError } from '../errors.js';
import { defineConfigSchema, type ConfigSchema, type Infer } from './schema.js';

export type {
  ArraySchemaOptions,
  InferObject,
  InferShape,
  ObjectSchemaOptions,
  RecordSchemaOptions,
  UnknownKeysPolicy,
};

/** 对象 schema 的字段表（沿用配置包历史命名）。 */
export type ConfigShape = SchemaShape;

/** 配置 schema 选项必须是普通对象，避免 null/数组在读取属性时抛原生异常。 */
function assertConfigOptions(name: string, value: unknown): void {
  if (!isPlainObject(value)) {
    throw new ConfigSchemaDefinitionError(`${name} 必须是普通对象（实际值: ${String(value)}）`);
  }
}

/**
 * 创建对象 schema。
 *
 * @param shape - 字段名到 schema 的映射
 * @param options - 未知键与缺失 section 策略；`missingSection` 缺省 `empty`
 * @returns 对象 schema
 * @throws {ConfigSchemaDefinitionError} 字段名非法或策略非法
 */
export function object<
  const TShape extends ConfigShape,
  const TOptions extends ObjectSchemaOptions = {},
>(shape: TShape, options: TOptions = {} as TOptions): ConfigSchema<InferObject<TShape, TOptions>> {
  assertConfigOptions('object options', options);
  return defineConfigSchema(() =>
    schemaObject(
      shape,
      options.missingSection === undefined ? { ...options, missingSection: 'empty' } : options,
    ),
  );
}

/**
 * 创建数组 schema（输出只读数组）。
 *
 * @param itemSchema - 元素 schema
 * @param options - 长度约束
 * @returns 数组 schema
 * @throws {ConfigSchemaDefinitionError} 选项非法
 */
export function array<TItem>(
  itemSchema: BaseSchema<TItem>,
  options: ArraySchemaOptions = {},
): ConfigSchema<readonly TItem[]> {
  return defineConfigSchema(() => schemaArray(itemSchema, options));
}

/**
 * 创建字符串键记录 schema。
 *
 * @param valueSchema - 所有值的 schema
 * @param options - 键正则与数量约束
 * @returns 记录 schema
 * @throws {ConfigSchemaDefinitionError} 选项非法
 */
export function record<TValue>(
  valueSchema: BaseSchema<TValue>,
  options: RecordSchemaOptions = {},
): ConfigSchema<Readonly<Record<string, TValue>>> {
  return defineConfigSchema(() => schemaRecord(valueSchema, options));
}

/**
 * 创建联合 schema（按声明顺序尝试）。
 *
 * @param options - 至少两个 schema
 * @returns 输出为各分支输出的联合
 * @throws {ConfigSchemaDefinitionError} 分支少于两个
 */
export function union<const TOptions extends readonly BaseSchema<unknown>[]>(
  options: TOptions,
): ConfigSchema<Infer<TOptions[number]>> {
  return defineConfigSchema(() => schemaUnion(options));
}
