/**
 * @file Logger 公共类型与内部配置/状态类型。
 * @description 只包含类型，不承载运行期逻辑；便于实现与测试分别引用。
 * @module log/logger-types
 */

import type { EmittedLogLevel, LogLevel } from './level.js';
import type { LogFields, LogRecord, RecordNormalizationOptions } from './record.js';
import type { LogSink } from './sink.js';
import type { LogSinkError } from './errors.js';

/** 字段提供者；函数形式仅在级别启用后求值，避免无效开销。 */
export type LogFieldsProvider = LogFields | (() => LogFields);

/** sink 失败上下文。 */
export interface LogSinkFailure {
  /** 已包装的 sink 错误。 */
  readonly error: LogSinkError;
  /** 正在写入的记录。 */
  readonly record: LogRecord;
  /** 出错的 sink。 */
  readonly sink: LogSink;
}

/** 日志器选项。 */
export interface LoggerOptions {
  /** 级别阈值，缺省 `info`；`silent` 关闭全部写入。 */
  readonly level?: LogLevel;
  /** 输出目标，缺省 console sink；空数组表示显式丢弃日志。 */
  readonly sinks?: readonly LogSink[];
  /** 根 bindings。 */
  readonly bindings?: LogFields;
  /** 时间源，缺省 `Date.now`。 */
  readonly clock?: () => number;
  /**
   * sink 写入失败处理器；未提供时同步失败直接抛出，异步失败由 `flush()`/`close()` 抛出。
   * `flush()`/`close()` 自身的 sink 失败总是聚合抛出，不经过该处理器。
   * 处理器自身同步抛错或异步 rejection 都会被包装为新的 `LogSinkError`。
   */
  readonly onSinkError?: (failure: LogSinkFailure) => void | Promise<void>;
  /** 最大序列化深度，缺省 8。 */
  readonly maxDepth?: number;
  /** 字段/键/默认 message 的字符串最大长度，缺省 8192。 */
  readonly maxStringLength?: number;
  /** 集合最大条目数，缺省 1000。 */
  readonly maxEntries?: number;
  /** 是否在错误字段中保留调用栈，缺省 true。 */
  readonly shouldIncludeErrorStack?: boolean;
  /** message 最大长度，缺省与 `maxStringLength` 相同。 */
  readonly maxMessageLength?: number;
}

/** 跨 runtime 日志器。 */
export interface Logger {
  /** 当前级别阈值。 */
  readonly level: LogLevel;
  /**
   * 判断级别是否达到阈值。
   *
   * @param level - 待写入级别
   * @returns 达到阈值时返回 true
   */
  isLevelEnabled(level: EmittedLogLevel): boolean;
  /**
   * 写入指定级别。
   *
   * @param level - 待写入级别
   * @param message - 人类可读消息
   * @param fields - 结构化字段或延迟求值函数
   * @throws {LogClosedError} 日志器已关闭
   * @throws {LogDefinitionError} 级别非法或时钟返回非有限值
   */
  log(level: EmittedLogLevel, message: string, fields?: LogFieldsProvider): void;
  /** 写入 trace 级别。 */
  /**
   * 写入 trace 级别。
   *
   * @param message - 人类可读消息
   * @param fields - 结构化字段或延迟求值函数
   */
  trace(message: string, fields?: LogFieldsProvider): void;
  /**
   * 写入 debug 级别。
   *
   * @param message - 人类可读消息
   * @param fields - 结构化字段或延迟求值函数
   */
  debug(message: string, fields?: LogFieldsProvider): void;
  /**
   * 写入 info 级别。
   *
   * @param message - 人类可读消息
   * @param fields - 结构化字段或延迟求值函数
   */
  info(message: string, fields?: LogFieldsProvider): void;
  /**
   * 写入 warn 级别。
   *
   * @param message - 人类可读消息
   * @param fields - 结构化字段或延迟求值函数
   */
  warn(message: string, fields?: LogFieldsProvider): void;
  /**
   * 写入 error 级别。
   *
   * @param message - 人类可读消息
   * @param fields - 结构化字段或延迟求值函数
   */
  error(message: string, fields?: LogFieldsProvider): void;
  /**
   * 写入 fatal 级别。
   *
   * @param message - 人类可读消息
   * @param fields - 结构化字段或延迟求值函数
   */
  fatal(message: string, fields?: LogFieldsProvider): void;
  /**
   * 创建带附加 bindings 的子 logger；共享 sink 与生命周期。
   *
   * @param bindings - 追加的绑定字段
   * @returns 新的 Logger 实例
   */
  child(bindings?: LogFields): Logger;
  /**
   * 返回替换级别阈值的 logger；共享 sink 与 bindings。
   *
   * @param level - 新阈值
   * @returns 新的 Logger 实例
   * @throws {LogDefinitionError} 级别非法
   */
  withLevel(level: LogLevel): Logger;
  /**
   * 等待在飞行写入与 sink 缓冲刷新。
   *
   * @returns 刷新完成时 resolve
   * @throws {AggregateError} sink 刷新失败
   */
  flush(): Promise<void>;
  /**
   * 刷新并关闭全部 sink；幂等。
   *
   * @returns 关闭完成时 resolve
   * @throws {AggregateError} sink 刷新或关闭失败
   */
  close(): Promise<void>;
}

/** 内部解析后的配置。 */
export interface LoggerConfig {
  readonly level: LogLevel;
  readonly sinks: readonly LogSink[];
  readonly clock: () => number;
  readonly onSinkError: ((failure: LogSinkFailure) => void | Promise<void>) | undefined;
  readonly normalization: RecordNormalizationOptions;
  readonly maxMessageLength: number;
}

/** Logger 生命周期状态。 */
export type LoggerLifecycle =
  | { readonly status: 'open' }
  | { readonly status: 'closing'; readonly promise: Promise<void> }
  | { readonly status: 'closed'; readonly promise: Promise<void> };

/** Logger 共享状态。 */
export interface LoggerState {
  readonly pending: Set<Promise<void>>;
  readonly errors: LogSinkError[];
  lifecycle: LoggerLifecycle;
}
