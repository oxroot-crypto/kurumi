/**
 * @file JSON 安全序列化与错误序列化的公共入口。
 * @description 聚合 shared/values/errors 子模块，对上层保持稳定的 `serialize.ts` API。
 * @module core/serialize
 */

import { deepFreeze, type JsonValue } from '../value.js';
import { serializeErrorValue } from './errors.js';
import {
  isErrorLike,
  resolveOptions,
  truncateString,
  type SerializeErrorOptions,
  type SerializedError,
} from './shared.js';
import { convertSafely, toJsonSafeValue } from './values.js';

export {
  DEFAULT_SERIALIZE_DEPTH,
  DEFAULT_SERIALIZE_ENTRIES,
  DEFAULT_SERIALIZE_STRING_LENGTH,
  isErrorLike,
  type ErrorLike,
  type SerializeErrorOptions,
  type SerializedError,
  type SerializeValueOptions,
} from './shared.js';
export { toJsonSafeValue } from './values.js';

/**
 * 将错误对象序列化为稳定结构。
 *
 * @remarks
 * - 默认保留 `stack`、`cause`、`code` 与自身可枚举字段。
 * - `AggregateError.errors` 逐项序列化；非 Error 输入包装为 `NonError`。
 * - 循环 cause 输出 `[Circular]`，超过深度输出 `[MaxDepth]`。
 *
 * @param error - 任意错误值
 * @param options - 深度、长度、条目与调用栈选项
 * @returns 可安全 JSON 序列化且递归冻结的错误结构
 * @throws {KurumiError} 选项不是合法安全整数或布尔值
 *
 * @example
 * ```ts
 * import { serializeError } from '@kurumi/core';
 * serializeError(new Error('boom', { cause: new Error('root') }));
 * // { name: 'Error', message: 'boom', stack: '...', cause: { ... } }
 * ```
 */
export function serializeError(
  error: unknown,
  options: SerializeErrorOptions = {},
): SerializedError {
  const resolved = resolveOptions(options, true);
  if (!isErrorLike(error)) {
    return Object.freeze({
      name: 'NonError',
      message: truncateString(describeNonError(error), resolved.maxStringLength),
    });
  }
  return deepFreeze(
    serializeErrorValue(error, {
      depth: 0,
      ancestors: new WeakSet<object>(),
      options: resolved,
      convertSafely,
    }),
  );
}

/** 为非错误值生成可诊断描述。 */
function describeNonError(value: unknown): string {
  if (typeof value === 'string') return value;
  if (value === undefined) return 'undefined';
  try {
    const safe = toJsonSafeValue(value, { maxDepth: 2 });
    return typeof safe === 'string' ? safe : (JSON.stringify(safe) ?? '[Unserializable]');
  } catch {
    return '[Unserializable]';
  }
}

// 保持 JsonValue 在声明文件中可见（供下游类型推导使用）。
export type { JsonValue };
