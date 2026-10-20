/**
 * @file 序列化共享类型、选项解析与低层工具。
 * @description 仅服务于 serialize 子模块；对外的公开 API 由 serialize/index.ts 统一导出。
 * @module core/serialize/shared
 */

import { KurumiError } from '../errors.js';
import type { JsonValue } from '../value.js';

/** 默认递归深度，超过后输出 `[MaxDepth]`。 */
export const DEFAULT_SERIALIZE_DEPTH = 8;

/** 默认字符串截断长度。 */
export const DEFAULT_SERIALIZE_STRING_LENGTH = 8_192;

/** 默认数组/对象/Map/Set 条目上限。 */
export const DEFAULT_SERIALIZE_ENTRIES = 1_000;

/** 序列化值选项。 */
export interface SerializeValueOptions {
  /** 最大递归深度；缺省 8，允许 0。 */
  readonly maxDepth?: number;
  /** 字符串最大长度；缺省 8192，必须 >= 1。 */
  readonly maxStringLength?: number;
  /** 集合最大条目数；缺省 1000，必须 >= 1。 */
  readonly maxEntries?: number;
  /** 是否保留 Error 的 `stack`；缺省 false。 */
  readonly shouldIncludeStack?: boolean;
}

/** 错误序列化选项。 */
export interface SerializeErrorOptions extends SerializeValueOptions {}

/** 结构上可识别的错误对象。 */
export type ErrorLike = object & {
  /** 错误名称。 */
  readonly name: string;
  /** 错误描述。 */
  readonly message: string;
  /** 调用栈。 */
  readonly stack?: string;
  /** 原始错误。 */
  readonly cause?: unknown;
};

/** 可安全写入日志或上报的错误结构。 */
export interface SerializedError {
  /** 错误名称。 */
  readonly name: string;
  /** 错误描述。 */
  readonly message: string;
  /** 调用栈；仅在 `shouldIncludeStack` 为 true 且原错误存在时出现。 */
  readonly stack?: string;
  /** 错误码；原错误存在该字段时出现。 */
  readonly code?: JsonValue;
  /** 因果链；循环引用为 `[Circular]`，超深为 `[MaxDepth]`。 */
  readonly cause?: JsonValue;
  /** `AggregateError` 的子错误数组。 */
  readonly errors?: JsonValue;
  /** 原错误自身的可枚举字段。 */
  readonly [key: string]: JsonValue;
}

/** 解析后的序列化选项。 */
export interface ResolvedSerializeOptions {
  readonly maxDepth: number;
  readonly maxStringLength: number;
  readonly maxEntries: number;
  readonly shouldIncludeStack: boolean;
}

/** 序列化错误时可变的结果结构。 */
export interface MutableSerializedError {
  name: string;
  message: string;
  stack?: string;
  code?: JsonValue;
  cause?: JsonValue;
  errors?: JsonValue;
  [key: string]: JsonValue;
}

/** 将任意值安全转换为 JSON 值的回调类型。 */
export type ConvertSafely = (
  value: unknown,
  depth: number,
  ancestors: WeakSet<object>,
  options: ResolvedSerializeOptions,
) => JsonValue;

/** 错误序列化上下文，避免 helper 参数超过 4 个。 */
export interface ErrorSerializeContext {
  /** 当前递归深度。 */
  readonly depth: number;
  /** 当前祖先集合，用于检测循环引用。 */
  readonly ancestors: WeakSet<object>;
  /** 解析后的序列化选项。 */
  readonly options: ResolvedSerializeOptions;
  /** 将任意值转换为 JSON 值的回调。 */
  readonly convertSafely: ConvertSafely;
}

/**
 * 判断值是否具备 `name` 与 `message` 字符串字段。
 *
 * @param value - 待判断值
 * @returns 是错误形态时返回 true
 *
 * @example
 * ```ts
 * import { isErrorLike } from '@kurumi/core';
 * isErrorLike(new Error('boom')); // true
 * ```
 */
export function isErrorLike(value: unknown): value is ErrorLike {
  if (typeof value !== 'object' || value === null) return false;
  try {
    return (
      typeof Reflect.get(value, 'name') === 'string' &&
      typeof Reflect.get(value, 'message') === 'string'
    );
  } catch {
    return false;
  }
}

/** 判断值是否为非 null 对象。 */
export function isObject(value: unknown): value is object {
  return typeof value === 'object' && value !== null;
}

/**
 * 解析并校验序列化选项。
 *
 * @param options - 调用方传入的选项
 * @param defaultIncludeStack - 未显式指定时是否保留调用栈
 * @returns 已校验的选项
 * @throws {KurumiError} 选项不是合法安全整数或布尔值
 */
export function resolveOptions(
  options: SerializeValueOptions,
  defaultIncludeStack: boolean,
): ResolvedSerializeOptions {
  const maxDepth = options.maxDepth ?? DEFAULT_SERIALIZE_DEPTH;
  const maxStringLength = options.maxStringLength ?? DEFAULT_SERIALIZE_STRING_LENGTH;
  const maxEntries = options.maxEntries ?? DEFAULT_SERIALIZE_ENTRIES;
  assertInteger('maxDepth', maxDepth, 0);
  assertInteger('maxStringLength', maxStringLength, 1);
  assertInteger('maxEntries', maxEntries, 1);
  if (options.shouldIncludeStack !== undefined && typeof options.shouldIncludeStack !== 'boolean') {
    throw new KurumiError('序列化选项 shouldIncludeStack 必须是布尔值', {
      code: 'invalid_options',
    });
  }
  return {
    maxDepth,
    maxStringLength,
    maxEntries,
    shouldIncludeStack: options.shouldIncludeStack ?? defaultIncludeStack,
  };
}

/**
 * 校验安全整数选项。
 *
 * @param name - 选项名
 * @param value - 待校验值
 * @param minimum - 允许的最小值
 * @throws {KurumiError} 值不是安全整数或小于最小值
 */
function assertInteger(name: string, value: number, minimum: number): void {
  if (!Number.isSafeInteger(value) || value < minimum) {
    throw new KurumiError(
      `序列化选项 ${name} 必须 >= ${minimum} 的安全整数，实际为 ${String(value)}`,
      { code: 'invalid_options' },
    );
  }
}

/**
 * 按最大长度截断字符串。
 *
 * @param value - 原始字符串
 * @param maxLength - 最大字符数
 * @returns 原字符串或带截断标记的新字符串
 */
export function truncateString(value: string, maxLength: number): string {
  if (value.length <= maxLength) return value;
  const suffix = '…[truncated]';
  // 上限过小时无法同时容纳内容与标记；优先保证输出不超过 maxLength。
  if (maxLength <= suffix.length) return value.slice(0, maxLength);
  return `${value.slice(0, maxLength - suffix.length)}${suffix}`;
}

/**
 * 读取对象属性；Proxy/getter 抛错时返回可诊断占位符。
 *
 * @param value - 源对象
 * @param key - 属性名
 * @returns 属性值或 `[Unreadable]`
 */
export function readProperty(value: object, key: string): unknown {
  try {
    return Reflect.get(value, key);
  } catch {
    return '[Unreadable]';
  }
}
