/**
 * @file Logger 选项解析、错误包装与记录辅助函数。
 * @description 不持有状态；供 DefaultLogger 与 createLogger 复用。
 * @module log/logger-internals
 */

import {
  DEFAULT_SERIALIZE_DEPTH,
  DEFAULT_SERIALIZE_ENTRIES,
  DEFAULT_SERIALIZE_STRING_LENGTH,
  deepFreeze,
  toJsonSafeValue,
} from '@kurumi/core';

import { LogDefinitionError, LogSinkError } from './errors.js';
import { LogLevel, isLogLevel } from './level.js';
import {
  normalizeLogFields,
  type LogFields,
  type RecordNormalizationOptions,
  type ResolvedLogFields,
} from './record.js';
import { createConsoleSink } from './sink/console.js';
import type { LogSink } from './sink.js';
import type { LoggerConfig, LoggerOptions, LogSinkFailure } from './logger-types.js';

/**
 * 解析并校验级别阈值。
 *
 * @param level - 调用方传入的级别
 * @returns 合法级别
 * @throws {LogDefinitionError} 级别非法
 */
export function resolveLevel(level: LogLevel | undefined): LogLevel {
  const resolved = level ?? LogLevel.Info;
  if (!isLogLevel(resolved)) {
    throw new LogDefinitionError(`非法日志级别: ${String(resolved)}`);
  }
  return resolved;
}

/**
 * 解析并冻结 sink 列表。
 *
 * @param sinks - 调用方传入的 sink 列表
 * @returns 冻结后的 sink 数组
 * @throws {LogDefinitionError} 不是数组或某个 sink 缺少 write()
 */
export function resolveSinks(sinks: readonly LogSink[] | undefined): readonly LogSink[] {
  const resolved = sinks ?? [createConsoleSink()];
  if (!Array.isArray(resolved)) {
    throw new LogDefinitionError(`sinks 必须是数组（实际值: ${String(resolved)}）`);
  }
  for (const [index, sink] of resolved.entries()) {
    if (typeof sink !== 'object' || sink === null || typeof sink.write !== 'function') {
      throw new LogDefinitionError(`第 ${index + 1} 个 sink 必须实现 write(record)`);
    }
  }
  return Object.freeze([...resolved]);
}

/**
 * 解析时钟函数。
 *
 * @param clock - 可选时钟
 * @returns 可调用的时钟
 * @throws {LogDefinitionError} 不是函数
 */
export function resolveClock(clock: (() => number) | undefined): () => number {
  const resolved = clock ?? Date.now;
  if (typeof resolved !== 'function') {
    throw new LogDefinitionError(`clock 必须是函数（实际值: ${String(resolved)}）`);
  }
  return resolved;
}

/**
 * 解析 sink 错误处理器。
 *
 * @param handler - 可选处理器
 * @returns 原处理器或 undefined
 * @throws {LogDefinitionError} 不是函数
 */
export function resolveSinkErrorHandler(
  handler: ((failure: LogSinkFailure) => void | Promise<void>) | undefined,
): ((failure: LogSinkFailure) => void | Promise<void>) | undefined {
  if (handler === undefined) return undefined;
  if (typeof handler !== 'function') {
    throw new LogDefinitionError(`onSinkError 必须是函数（实际值: ${String(handler)}）`);
  }
  return handler;
}

/**
 * 解析 message 最大长度。
 *
 * @param value - 调用方配置
 * @param fallback - 缺省长度
 * @returns 已校验的最大长度
 * @throws {LogDefinitionError} 不是 >= 1 的安全整数
 */
export function resolveMessageLength(value: number | undefined, fallback: number): number {
  if (value === undefined) return fallback;
  assertInteger('maxMessageLength', value, 1);
  return value;
}

/**
 * 解析字段归一化选项。
 *
 * @param options - Logger 选项
 * @returns 已校验的归一化选项
 * @throws {LogDefinitionError} 选项非法
 */
export function resolveNormalization(options: LoggerOptions): RecordNormalizationOptions {
  const maxDepth = options.maxDepth ?? DEFAULT_SERIALIZE_DEPTH;
  const maxStringLength = options.maxStringLength ?? DEFAULT_SERIALIZE_STRING_LENGTH;
  const maxEntries = options.maxEntries ?? DEFAULT_SERIALIZE_ENTRIES;
  assertInteger('maxDepth', maxDepth, 0);
  assertInteger('maxStringLength', maxStringLength, 1);
  assertInteger('maxEntries', maxEntries, 1);
  const shouldIncludeStack = options.shouldIncludeErrorStack ?? true;
  if (typeof shouldIncludeStack !== 'boolean') {
    throw new LogDefinitionError(
      `shouldIncludeErrorStack 必须是布尔值（实际值: ${String(shouldIncludeStack)}）`,
    );
  }
  return { maxDepth, maxStringLength, maxEntries, shouldIncludeStack };
}

/** 校验安全整数选项。 */
function assertInteger(name: string, value: number, minimum: number): void {
  if (!Number.isSafeInteger(value) || value < minimum) {
    throw new LogDefinitionError(`${name} 必须 >= ${minimum} 的安全整数，实际为 ${String(value)}`);
  }
}

/**
 * 组装解析后的 Logger 配置。
 *
 * @param options - Logger 选项
 * @returns 已解析配置
 */
export function resolveLoggerConfig(options: LoggerOptions): LoggerConfig {
  const normalization = resolveNormalization(options);
  return {
    level: resolveLevel(options.level),
    sinks: resolveSinks(options.sinks),
    clock: resolveClock(options.clock),
    onSinkError: resolveSinkErrorHandler(options.onSinkError),
    normalization,
    maxMessageLength: resolveMessageLength(options.maxMessageLength, normalization.maxStringLength),
  };
}

/**
 * 解析字段提供者；求值失败时降级为可诊断字段。
 *
 * @param fields - 字段或字段函数
 * @param options - 归一化选项
 * @returns 归一化字段或 undefined
 */
export function resolveFields(
  fields: LogFields | (() => LogFields) | undefined,
  options: RecordNormalizationOptions,
): ResolvedLogFields | undefined {
  if (fields === undefined) return undefined;
  try {
    const value = typeof fields === 'function' ? fields() : fields;
    const normalized = normalizeLogFields(value, options);
    return Object.keys(normalized).length === 0 ? undefined : normalized;
  } catch (cause) {
    // 日志字段求值失败不能反向中断业务；把异常降级成可诊断字段。
    return deepFreeze({ '[fields_error]': toJsonSafeValue(cause, options) });
  }
}

/**
 * 将任意 cause 包装为 sink 错误。
 *
 * @param sink - 出错的 sink
 * @param message - 失败描述
 * @param cause - 底层异常
 * @returns 结构化 sink 错误
 */
export function toSinkError(sink: LogSink, message: string, cause: unknown): LogSinkError {
  return new LogSinkError(sinkName(sink), message, cause);
}

/** 返回 sink 名称；匿名 sink 使用占位符。 */
export function sinkName(sink: LogSink): string {
  return typeof sink.name === 'string' && sink.name.trim() !== '' ? sink.name : '<anonymous>';
}

/** 将 cause 转为可读消息。 */
export function describeCause(cause: unknown): string {
  try {
    if (cause instanceof Error && cause.message !== '') return cause.message;
    return String(cause);
  } catch {
    return '[Unserializable]';
  }
}

/**
 * 聚合同步 sink 失败：单条失败返回该错误，多条返回 AggregateError。
 *
 * @param failures - 失败列表
 * @param message - 聚合错误消息
 * @returns 单个错误或聚合错误
 */
export function aggregateFailures(
  failures: readonly LogSinkError[],
  message: string,
): LogSinkError | AggregateError {
  const first = failures[0];
  if (failures.length === 1 && first !== undefined) return first;
  return new AggregateError(failures, message);
}

/** 判断两个 sink 失败是否来自同一个 cause。 */
export function sameFailureCause(left: LogSinkError, right: LogSinkError): boolean {
  return left.sinkName === right.sinkName && left.cause !== undefined && left.cause === right.cause;
}

/** 追加失败；对同一 cause 去重，避免 flush 重复上报。 */
export function addFailure(target: LogSinkError[], failure: LogSinkError): void {
  if (!target.some((existing) => sameFailureCause(existing, failure))) target.push(failure);
}

/** 将 flush/close 抛出的聚合错误展开为失败列表。 */
export function appendAggregateFailures(target: LogSinkError[], cause: unknown): void {
  if (cause instanceof LogSinkError) {
    target.push(cause);
    return;
  }
  if (cause instanceof AggregateError) {
    for (const item of cause.errors) {
      target.push(
        item instanceof LogSinkError
          ? item
          : new LogSinkError('<flush>', describeCause(item), item),
      );
    }
    return;
  }
  target.push(new LogSinkError('<flush>', describeCause(cause), cause));
}

/**
 * 判断值是否为 PromiseLike。
 *
 * @param value - 待判断值
 * @returns 具备 then 函数时返回 true
 */
export function isPromiseLike(value: unknown): value is PromiseLike<void> {
  if (typeof value !== 'object' || value === null) return false;
  try {
    return typeof Reflect.get(value, 'then') === 'function';
  } catch {
    return false;
  }
}

/**
 * 截断日志消息。
 *
 * @param message - 原始消息
 * @param maxLength - 最大字符数
 * @returns 原消息或带截断标记的消息
 */
export function truncateMessage(message: string, maxLength: number): string {
  if (message.length <= maxLength) return message;
  const suffix = '…[truncated]';
  if (maxLength <= suffix.length) return message.slice(0, maxLength);
  return `${message.slice(0, maxLength - suffix.length)}${suffix}`;
}
