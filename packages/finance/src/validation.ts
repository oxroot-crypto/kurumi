/**
 * @file 金融包边界校验适配器。
 * @description 通用校验来自 `@kurumi/core`，Decimal 校验来自 `@kurumi/math`；本文件只负责绑定金融领域错误。
 * @module finance/validation
 */

import {
  assertArrayMaxLength as coreAssertArrayMaxLength,
  assertBoolean as coreAssertBoolean,
  assertFiniteNumber as coreAssertFiniteNumber,
  assertNonEmptyString as coreAssertNonEmptyString,
  assertOneOf as coreAssertOneOf,
  assertPattern as coreAssertPattern,
  assertPlainObject as coreAssertPlainObject,
  assertPositiveSafeInteger as coreAssertPositiveSafeInteger,
  assertRange as coreAssertRange,
  assertSafeInteger as coreAssertSafeInteger,
  assertString as coreAssertString,
  assertTimestampMs as coreAssertTimestampMs,
  describeValue as coreDescribeValue,
  type ValidationErrorFactory,
} from '@kurumi/core';
import {
  assertDecimal as mathAssertDecimal,
  assertNonNegativeDecimal as mathAssertNonNegativeDecimal,
  assertPositiveDecimal as mathAssertPositiveDecimal,
  type Decimal,
} from '@kurumi/math';

import { FinanceDefinitionError } from './errors.js';

export type { ValidationErrorFactory };

/** 金融定义期错误工厂。 */
const financeError: ValidationErrorFactory = (message) => new FinanceDefinitionError(message);

/**
 * 校验字符串。
 *
 * @param value - 待校验值
 * @param path - 字段路径
 * @param createError - 错误工厂
 */
export function assertString(
  value: unknown,
  path: string,
  createError: ValidationErrorFactory = financeError,
): asserts value is string {
  coreAssertString(value, path, createError);
}

/**
 * 校验非空白字符串。
 *
 * @param value - 待校验值
 * @param path - 字段路径
 * @param createError - 错误工厂
 */
export function assertNonEmptyString(
  value: unknown,
  path: string,
  createError: ValidationErrorFactory = financeError,
): asserts value is string {
  coreAssertNonEmptyString(value, path, createError);
}

/**
 * 校验字符串是否匹配正则。
 *
 * @param value - 待校验字符串
 * @param pattern - 正则
 * @param path - 字段路径
 * @param createError - 错误工厂
 */
export function assertPattern(
  value: string,
  pattern: RegExp,
  path: string,
  createError: ValidationErrorFactory = financeError,
): void {
  coreAssertPattern(value, pattern, path, createError);
}

/**
 * 校验非负安全整数。
 *
 * @param value - 待校验值
 * @param path - 字段路径
 * @param createError - 错误工厂
 */
export function assertSafeInteger(
  value: unknown,
  path: string,
  createError: ValidationErrorFactory = financeError,
): asserts value is number {
  coreAssertSafeInteger(value, path, createError);
}

/**
 * 校验正安全整数。
 *
 * @param value - 待校验值
 * @param path - 字段路径
 * @param createError - 错误工厂
 */
export function assertPositiveSafeInteger(
  value: unknown,
  path: string,
  createError: ValidationErrorFactory = financeError,
): asserts value is number {
  coreAssertPositiveSafeInteger(value, path, createError);
}

/**
 * 校验 epoch 毫秒时间戳。
 *
 * @param value - 待校验值
 * @param path - 字段路径
 * @param createError - 错误工厂
 */
export function assertTimestampMs(
  value: unknown,
  path: string,
  createError: ValidationErrorFactory = financeError,
): asserts value is number {
  coreAssertTimestampMs(value, path, createError);
}

/**
 * 校验有限数值。
 *
 * @param value - 待校验值
 * @param path - 字段路径
 * @param createError - 错误工厂
 */
export function assertFiniteNumber(
  value: unknown,
  path: string,
  createError: ValidationErrorFactory = financeError,
): asserts value is number {
  coreAssertFiniteNumber(value, path, createError);
}

/**
 * 校验布尔值。
 *
 * @param value - 待校验值
 * @param path - 字段路径
 * @param createError - 错误工厂
 */
export function assertBoolean(
  value: unknown,
  path: string,
  createError: ValidationErrorFactory = financeError,
): asserts value is boolean {
  coreAssertBoolean(value, path, createError);
}

/**
 * 校验值属于封闭字符串集合。
 *
 * @param value - 待校验值
 * @param allowed - 允许值
 * @param path - 字段路径
 * @param createError - 错误工厂
 */
export function assertOneOf<TValue extends string>(
  value: unknown,
  allowed: readonly TValue[],
  path: string,
  createError: ValidationErrorFactory = financeError,
): asserts value is TValue {
  coreAssertOneOf(value, allowed, path, createError);
}

/**
 * 校验普通对象。
 *
 * @param value - 待校验值
 * @param path - 字段路径
 * @param createError - 错误工厂
 */
export function assertPlainObject(
  value: unknown,
  path: string,
  createError: ValidationErrorFactory = financeError,
): asserts value is Record<string, unknown> {
  coreAssertPlainObject(value, path, createError);
}

/**
 * 校验数组长度上限。
 *
 * @param value - 待校验数组
 * @param path - 字段路径
 * @param maxLength - 最大长度
 * @param createError - 错误工厂
 */
export function assertArrayMaxLength(
  value: readonly unknown[],
  path: string,
  maxLength: number,
  createError: ValidationErrorFactory = financeError,
): void {
  coreAssertArrayMaxLength(value, path, maxLength, createError);
}

/**
 * 校验数值范围（闭区间）。
 *
 * @param value - 待校验值
 * @param path - 字段路径
 * @param min - 下界
 * @param max - 上界
 * @param createError - 错误工厂
 */
export function assertRange(
  value: number,
  path: string,
  min: number,
  max: number,
  createError: ValidationErrorFactory = financeError,
): void {
  coreAssertRange(value, path, min, max, createError);
}

/**
 * 校验值为 Decimal。
 *
 * @param value - 待校验值
 * @param path - 字段路径
 * @param createError - 错误工厂
 */
export function assertDecimal(
  value: unknown,
  path: string,
  createError: ValidationErrorFactory = financeError,
): asserts value is Decimal {
  mathAssertDecimal(value, path, createError);
}

/**
 * 校验值为正 Decimal。
 *
 * @param value - 待校验值
 * @param path - 字段路径
 * @param createError - 错误工厂
 */
export function assertPositiveDecimal(
  value: unknown,
  path: string,
  createError: ValidationErrorFactory = financeError,
): asserts value is Decimal {
  mathAssertPositiveDecimal(value, path, createError);
}

/**
 * 校验值为非负 Decimal。
 *
 * @param value - 待校验值
 * @param path - 字段路径
 * @param createError - 错误工厂
 */
export function assertNonNegativeDecimal(
  value: unknown,
  path: string,
  createError: ValidationErrorFactory = financeError,
): asserts value is Decimal {
  mathAssertNonNegativeDecimal(value, path, createError);
}

/**
 * 把任意值描述为简短消息。
 *
 * @param value - 待描述值
 * @returns 最多 80 字符的描述
 */
export function describeValue(value: unknown): string {
  return coreDescribeValue(value);
}
