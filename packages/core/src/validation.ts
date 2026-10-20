/**
 * @file 通用边界校验工具。
 * @description 只依赖 core 自身；调用方可注入领域错误工厂，把同一套校验映射为 Finance/Math 等领域错误。
 * @module core/validation
 */

import { KurumiError } from './errors.js';
import { isPlainObject } from './value.js';

/** 根据调用场景构造错误。 */
export type ValidationErrorFactory = (message: string) => Error;

/** core 缺省校验错误。 */
export class CoreValidationError extends KurumiError {
  /**
   * @param message - 校验失败描述
   */
  constructor(message: string) {
    super(message, { code: 'core_validation_error' });
  }
}

/** 缺省错误工厂。 */
const defaultError: ValidationErrorFactory = (message) => new CoreValidationError(message);

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
  createError: ValidationErrorFactory = defaultError,
): asserts value is string {
  if (typeof value !== 'string') {
    throw createError(`${path} 必须是字符串（实际值: ${String(value)}）`);
  }
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
  createError: ValidationErrorFactory = defaultError,
): asserts value is string {
  assertString(value, path, createError);
  if (value.trim() === '') {
    throw createError(`${path} 不能为空或纯空白`);
  }
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
  createError: ValidationErrorFactory = defaultError,
): void {
  // `g` / `y` 正则的 test() 会修改 lastIndex；去掉状态标志，避免重复调用结果漂移。
  const stateless =
    pattern.global || pattern.sticky
      ? new RegExp(pattern.source, pattern.flags.replace(/[gy]/gu, ''))
      : pattern;
  if (!stateless.test(value)) {
    throw createError(`${path} 不匹配模式 ${pattern.source}（实际值: ${value}）`);
  }
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
  createError: ValidationErrorFactory = defaultError,
): asserts value is number {
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0) {
    throw createError(`${path} 必须是非负安全整数（实际值: ${String(value)}）`);
  }
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
  createError: ValidationErrorFactory = defaultError,
): asserts value is number {
  assertSafeInteger(value, path, createError);
  if (value <= 0) {
    throw createError(`${path} 必须是正安全整数（实际值: ${String(value)}）`);
  }
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
  createError: ValidationErrorFactory = defaultError,
): asserts value is number {
  assertSafeInteger(value, path, createError);
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
  createError: ValidationErrorFactory = defaultError,
): asserts value is number {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    throw createError(`${path} 必须是有限数值（实际值: ${String(value)}）`);
  }
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
  createError: ValidationErrorFactory = defaultError,
): asserts value is boolean {
  if (typeof value !== 'boolean') {
    throw createError(`${path} 必须是布尔值（实际值: ${String(value)}）`);
  }
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
  createError: ValidationErrorFactory = defaultError,
): asserts value is TValue {
  if (typeof value !== 'string' || !allowed.includes(value as TValue)) {
    throw createError(
      `${path} 必须是 ${allowed.map((item) => `"${item}"`).join(' | ')} 之一（实际值: ${String(value)}）`,
    );
  }
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
  createError: ValidationErrorFactory = defaultError,
): asserts value is Record<string, unknown> {
  if (!isPlainObject(value)) {
    throw createError(`${path} 必须是普通对象（实际值: ${String(value)}）`);
  }
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
  createError: ValidationErrorFactory = defaultError,
): void {
  if (value.length > maxLength) {
    throw createError(`${path} 长度 ${value.length} 超过上限 ${maxLength}`);
  }
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
  createError: ValidationErrorFactory = defaultError,
): void {
  if (!Number.isFinite(min) || !Number.isFinite(max) || min > max) {
    throw createError(`${path} 的范围定义非法（min=${String(min)}, max=${String(max)}）`);
  }
  if (!Number.isFinite(value) || value < min || value > max) {
    throw createError(`${path} 必须在 [${min}, ${max}] 内（实际值: ${String(value)}）`);
  }
}

/**
 * 把任意值描述为简短消息，避免超长错误文本。
 *
 * @param value - 待描述值
 * @returns 最多 80 字符的描述
 */
export function describeValue(value: unknown): string {
  let text: string;
  try {
    text = typeof value === 'string' ? value : String(value);
  } catch {
    // 对象可能没有可用的 toString/valueOf（如 Object.create(null)）；诊断信息不能反向抛错。
    text = '[Unserializable]';
  }
  return text.length > 80 ? `${text.slice(0, 80)}…` : text;
}
