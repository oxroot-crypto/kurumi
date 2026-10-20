/**
 * @file Error 对象的稳定序列化实现。
 * @description 只依赖 shared 层；通过上下文的 convertSafely 回调序列化任意字段，避免循环依赖。
 * @module core/serialize/errors
 */

import type { JsonValue } from '../value.js';
import { isForbiddenKey } from '../value.js';
import {
  isErrorLike,
  isObject,
  readProperty,
  truncateString,
  type ErrorLike,
  type ErrorSerializeContext,
  type MutableSerializedError,
  type SerializedError,
} from './shared.js';

/**
 * 将错误对象序列化为稳定结构。
 *
 * @param error - 已确认具备 name/message 的错误对象
 * @param context - 深度、祖先集合、选项与安全转换回调
 * @returns 可 JSON 序列化的错误结构
 */
export function serializeErrorValue(
  error: ErrorLike,
  context: ErrorSerializeContext,
): SerializedError {
  context.ancestors.add(error);
  try {
    const result = createErrorShell(error, context);
    writeErrorCode(result, error, context);
    writeErrorCause(result, error, context);
    writeAggregateErrors(result, error, context);
    writeErrorProperties(result, error, context);
    return result;
  } finally {
    context.ancestors.delete(error);
  }
}

/** 创建只含 name/message/stack 的错误外壳。 */
function createErrorShell(
  error: ErrorLike,
  context: ErrorSerializeContext,
): MutableSerializedError {
  const result: MutableSerializedError = {
    name: truncateString(readText(error, 'name') ?? 'Error', context.options.maxStringLength),
    message: truncateString(readText(error, 'message') ?? '', context.options.maxStringLength),
  };
  const stack = readText(error, 'stack');
  if (context.options.shouldIncludeStack && stack !== undefined) {
    result.stack = truncateString(stack, context.options.maxStringLength);
  }
  return result;
}

/** 写入错误码。 */
function writeErrorCode(
  result: MutableSerializedError,
  error: ErrorLike,
  context: ErrorSerializeContext,
): void {
  const code = readProperty(error, 'code');
  if (code !== undefined) {
    result.code = context.convertSafely(
      code,
      context.depth + 1,
      context.ancestors,
      context.options,
    );
  }
}

/** 写入 cause；保留循环与深度语义。 */
function writeErrorCause(
  result: MutableSerializedError,
  error: ErrorLike,
  context: ErrorSerializeContext,
): void {
  const cause = readProperty(error, 'cause');
  if (cause === undefined) return;
  if (context.depth >= context.options.maxDepth) {
    result.cause = '[MaxDepth]';
    return;
  }
  if (isObject(cause) && context.ancestors.has(cause)) {
    result.cause = '[Circular]';
    return;
  }
  result.cause = isErrorLike(cause)
    ? serializeErrorValue(cause, {
        ...context,
        depth: context.depth + 1,
      })
    : context.convertSafely(cause, context.depth + 1, context.ancestors, context.options);
}

/** 写入 AggregateError 的子错误。 */
function writeAggregateErrors(
  result: MutableSerializedError,
  error: ErrorLike,
  context: ErrorSerializeContext,
): void {
  const aggregate = readAggregateErrors(error);
  if (aggregate === undefined) return;
  if (context.depth >= context.options.maxDepth) {
    result.errors = '[MaxDepth]';
    return;
  }

  const limit = Math.min(aggregate.length, context.options.maxEntries);
  const errors: JsonValue[] = [];
  for (let index = 0; index < limit; index += 1) {
    const item = aggregate[index];
    if (isObject(item) && context.ancestors.has(item)) {
      errors.push('[Circular]');
    } else if (isErrorLike(item)) {
      errors.push(
        serializeErrorValue(item, {
          ...context,
          depth: context.depth + 1,
        }),
      );
    } else {
      errors.push(
        context.convertSafely(item, context.depth + 1, context.ancestors, context.options),
      );
    }
  }
  if (aggregate.length > limit) {
    errors.push(`[Truncated: ${aggregate.length - limit} more errors]`);
  }
  result.errors = errors;
}

/** 写入错误自身的可枚举字段。 */
function writeErrorProperties(
  result: MutableSerializedError,
  error: ErrorLike,
  context: ErrorSerializeContext,
): void {
  let keys: readonly string[];
  try {
    keys = Object.keys(error);
  } catch {
    return;
  }

  const limit = Math.min(keys.length, context.options.maxEntries);
  for (let index = 0; index < limit; index += 1) {
    const key = keys[index];
    if (key === undefined || isForbiddenKey(key) || isReservedErrorKey(key)) continue;
    const safeKey = truncateString(key, context.options.maxStringLength);
    result[safeKey] = context.convertSafely(
      readProperty(error, key),
      context.depth + 1,
      context.ancestors,
      context.options,
    );
  }
  if (keys.length > limit) {
    result['[truncated]'] = `[${keys.length - limit} more keys]`;
  }
}

/** 判断字段是否由错误外壳负责，避免重复写入。 */
function isReservedErrorKey(key: string): boolean {
  return (
    key === 'name' ||
    key === 'message' ||
    key === 'stack' ||
    key === 'cause' ||
    key === 'errors' ||
    key === 'code'
  );
}

/** 读取 AggregateError.errors；非数组返回 undefined。 */
function readAggregateErrors(error: ErrorLike): readonly unknown[] | undefined {
  try {
    if (typeof AggregateError !== 'undefined' && error instanceof AggregateError) {
      return error.errors;
    }
    const candidate = Reflect.get(error, 'errors');
    return Array.isArray(candidate) ? candidate : undefined;
  } catch {
    return undefined;
  }
}

/** 读取字符串字段。 */
function readText(error: ErrorLike, key: 'name' | 'message' | 'stack'): string | undefined {
  const value = readProperty(error, key);
  return typeof value === 'string' ? value : undefined;
}
