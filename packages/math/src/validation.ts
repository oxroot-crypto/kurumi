/**
 * @file 数学包边界校验：Decimal 专用断言与领域错误工厂。
 * @description 通用字符串/数值/对象校验由 `@kurumi/core` 提供，本文件只补齐 Decimal 语义。
 * @module math/validation
 */

import type { ValidationErrorFactory } from '@kurumi/core';

import { isDecimal, type Decimal } from './decimal.js';
import { MathDefinitionError } from './errors.js';

/** 数学定义期错误工厂。 */
export const mathValidationError: ValidationErrorFactory = (message) =>
  new MathDefinitionError(message);

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
  createError: ValidationErrorFactory = mathValidationError,
): asserts value is Decimal {
  if (!isDecimal(value)) {
    throw createError(`${path} 必须是 Decimal（实际值: ${String(value)}）`);
  }
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
  createError: ValidationErrorFactory = mathValidationError,
): asserts value is Decimal {
  assertDecimal(value, path, createError);
  if (!value.isPositive()) {
    throw createError(`${path} 必须大于 0（实际值: ${value.toString()}）`);
  }
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
  createError: ValidationErrorFactory = mathValidationError,
): asserts value is Decimal {
  assertDecimal(value, path, createError);
  if (value.isNegative()) {
    throw createError(`${path} 必须大于等于 0（实际值: ${value.toString()}）`);
  }
}
