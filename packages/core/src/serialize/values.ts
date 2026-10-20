/**
 * @file 任意值到 JSON 安全结构的转换实现。
 * @description 处理循环引用、深度、条目/字符串上限与特殊值；错误对象交给 errors 子模块。
 * @module core/serialize/values
 */

import { isForbiddenKey, type JsonValue } from '../value.js';
import { serializeErrorValue } from './errors.js';
import {
  isErrorLike,
  readProperty,
  resolveOptions,
  truncateString,
  type ErrorSerializeContext,
  type ResolvedSerializeOptions,
  type SerializeValueOptions,
} from './shared.js';

/**
 * 将任意值转换为 JSON 安全结构。
 *
 * @remarks
 * - 循环引用输出 `[Circular]`，超过深度输出 `[MaxDepth]`。
 * - `undefined`、`NaN`、`Infinity`、BigInt、Symbol、函数、Date、Map、Set 都有确定性表示。
 * - 危险键（`__proto__` / `prototype` / `constructor`）会被静默丢弃，防止原型污染。
 * - 集合超过 `maxEntries`、字符串超过 `maxStringLength` 时截断并附加标记。
 * - 不修改输入对象。
 *
 * @param value - 待转换的任意值
 * @param options - 深度、长度与条目上限
 * @returns 只包含 JSON 原始值、数组与普通对象的副本
 * @throws {KurumiError} 选项不是合法安全整数或布尔值
 *
 * @example
 * ```ts
 * import { toJsonSafeValue } from '@kurumi/core';
 * toJsonSafeValue({ at: new Date(0), count: 2n });
 * // { at: '1970-01-01T00:00:00.000Z', count: '2n' }
 * ```
 */
export function toJsonSafeValue(value: unknown, options: SerializeValueOptions = {}): JsonValue {
  const resolved = resolveOptions(options, false);
  return convertValue(value, 0, new WeakSet<object>(), resolved);
}

/**
 * 在捕获转换异常后返回 `[Unserializable]`。
 *
 * @param value - 待转换值
 * @param depth - 当前深度
 * @param ancestors - 祖先集合
 * @param options - 已解析选项
 * @returns JSON 安全值或异常占位符
 */
export function convertSafely(
  value: unknown,
  depth: number,
  ancestors: WeakSet<object>,
  options: ResolvedSerializeOptions,
): JsonValue {
  try {
    return convertValue(value, depth, ancestors, options);
  } catch {
    return '[Unserializable]';
  }
}

/** 转换值并兜底 Proxy/getter 异常。 */
function convertValue(
  value: unknown,
  depth: number,
  ancestors: WeakSet<object>,
  options: ResolvedSerializeOptions,
): JsonValue {
  if (depth > options.maxDepth) return '[MaxDepth]';
  try {
    return convertValueUnsafe(value, depth, ancestors, options);
  } catch {
    // 恶意 Proxy / 自定义 getter 不应让日志或诊断路径崩溃。
    return '[Unserializable]';
  }
}

/** 实际类型分派。 */
function convertValueUnsafe(
  value: unknown,
  depth: number,
  ancestors: WeakSet<object>,
  options: ResolvedSerializeOptions,
): JsonValue {
  if (value === null) return null;

  switch (typeof value) {
    case 'string':
      return truncateString(value, options.maxStringLength);
    case 'boolean':
    case 'number':
      return typeof value === 'number' ? convertNumber(value) : value;
    case 'bigint':
      return truncateString(`${value.toString()}n`, options.maxStringLength);
    case 'undefined':
      return null;
    case 'symbol':
      return truncateString(value.toString(), options.maxStringLength);
    case 'function':
      return truncateString(
        `[Function ${value.name === '' ? 'anonymous' : value.name}]`,
        options.maxStringLength,
      );
    case 'object':
      return convertObjectValue(value, depth, ancestors, options);
    default:
      return '[Unsupported]';
  }
}

/** 转换对象、数组、Map、Set、Date 与错误对象。 */
function convertObjectValue(
  value: object,
  depth: number,
  ancestors: WeakSet<object>,
  options: ResolvedSerializeOptions,
): JsonValue {
  if (ancestors.has(value)) return '[Circular]';
  if (isErrorLike(value)) {
    const errorContext: ErrorSerializeContext = {
      depth,
      ancestors,
      options,
      convertSafely,
    };
    return serializeErrorValue(value, errorContext);
  }
  if (value instanceof Date) {
    return Number.isNaN(value.getTime()) ? 'Invalid Date' : value.toISOString();
  }
  if (value instanceof RegExp) return truncateString(String(value), options.maxStringLength);
  if (Array.isArray(value)) return convertArray(value, depth, ancestors, options);
  if (value instanceof Map) return convertMap(value, depth, ancestors, options);
  if (value instanceof Set) return convertSet(value, depth, ancestors, options);
  return convertPlainObject(value, depth, ancestors, options);
}

/** 转换有限与非有限数值。 */
function convertNumber(value: number): JsonValue {
  if (Number.isFinite(value)) return value;
  if (Number.isNaN(value)) return 'NaN';
  return value > 0 ? 'Infinity' : '-Infinity';
}

/** 转换数组并应用条目上限。 */
function convertArray(
  value: readonly unknown[],
  depth: number,
  ancestors: WeakSet<object>,
  options: ResolvedSerializeOptions,
): JsonValue {
  ancestors.add(value);
  try {
    const limit = Math.min(value.length, options.maxEntries);
    const result: JsonValue[] = [];
    for (let index = 0; index < limit; index += 1) {
      result.push(convertSafely(value[index], depth + 1, ancestors, options));
    }
    if (value.length > limit) {
      result.push(`[Truncated: ${value.length - limit} more items]`);
    }
    return result;
  } finally {
    ancestors.delete(value);
  }
}

/** 转换 Map 并应用条目上限。 */
function convertMap(
  value: ReadonlyMap<unknown, unknown>,
  depth: number,
  ancestors: WeakSet<object>,
  options: ResolvedSerializeOptions,
): JsonValue {
  ancestors.add(value);
  try {
    const result: JsonValue[] = [];
    let count = 0;
    for (const [key, item] of value) {
      if (count >= options.maxEntries) {
        result.push(`[Truncated: ${value.size - count} more entries]`);
        break;
      }
      result.push([
        convertSafely(key, depth + 1, ancestors, options),
        convertSafely(item, depth + 1, ancestors, options),
      ]);
      count += 1;
    }
    return result;
  } finally {
    ancestors.delete(value);
  }
}

/** 转换 Set 并应用条目上限。 */
function convertSet(
  value: ReadonlySet<unknown>,
  depth: number,
  ancestors: WeakSet<object>,
  options: ResolvedSerializeOptions,
): JsonValue {
  ancestors.add(value);
  try {
    const result: JsonValue[] = [];
    let count = 0;
    for (const item of value) {
      if (count >= options.maxEntries) {
        result.push(`[Truncated: ${value.size - count} more items]`);
        break;
      }
      result.push(convertSafely(item, depth + 1, ancestors, options));
      count += 1;
    }
    return result;
  } finally {
    ancestors.delete(value);
  }
}

/** 转换普通/类实例对象并丢弃危险键。 */
function convertPlainObject(
  value: object,
  depth: number,
  ancestors: WeakSet<object>,
  options: ResolvedSerializeOptions,
): JsonValue {
  ancestors.add(value);
  try {
    let keys: readonly string[];
    try {
      keys = Object.keys(value);
    } catch {
      return '[Unserializable]';
    }

    const result: Record<string, JsonValue> = {};
    const limit = Math.min(keys.length, options.maxEntries);
    for (let index = 0; index < limit; index += 1) {
      const key = keys[index];
      if (key === undefined || isForbiddenKey(key)) continue;
      const safeKey = truncateString(key, options.maxStringLength);
      result[safeKey] = convertSafely(readProperty(value, key), depth + 1, ancestors, options);
    }
    if (keys.length > limit) {
      result['[truncated]'] = `[${keys.length - limit} more keys]`;
    }
    return result;
  } finally {
    ancestors.delete(value);
  }
}
