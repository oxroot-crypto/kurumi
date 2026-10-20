/**
 * @file Logger 工厂与默认实现。
 * @description 负责级别过滤、记录构建、sink 调度、异步错误收敛与关闭生命周期。
 * @module log/logger
 */

import { deepFreeze } from '@kurumi/core';

import { LogClosedError, LogDefinitionError, LogSinkError } from './errors.js';
import {
  LogLevel,
  isEmittedLogLevel,
  isLevelEnabled,
  isLogLevel,
  type EmittedLogLevel,
} from './level.js';
import {
  aggregateFailures,
  appendAggregateFailures,
  addFailure,
  describeCause,
  isPromiseLike,
  resolveFields,
  resolveLoggerConfig,
  toSinkError,
  truncateMessage,
} from './logger-internals.js';
import {
  type Logger,
  type LoggerConfig,
  type LoggerLifecycle,
  type LoggerOptions,
  type LoggerState,
  type LogFieldsProvider,
} from './logger-types.js';
import {
  mergeLogFields,
  normalizeLogFields,
  type LogFields,
  type LogRecord,
  type ResolvedLogFields,
} from './record.js';
import type { LogSink } from './sink.js';

export type { Logger, LoggerOptions, LogFieldsProvider, LogSinkFailure } from './logger-types.js';

/**
 * 创建日志器。
 *
 * @remarks
 * 默认输出到 console；日志器不读取配置、不修改全局状态。
 * 同一 logger 及其 `child()` 共享 sink 生命周期，任意一个实例 `close()` 都会关闭整组 sink。
 *
 * @param options - 级别、sink、bindings、时钟与序列化限制
 * @returns 已就绪的日志器
 * @throws {LogDefinitionError} 选项非法
 *
 * @example
 * ```ts
 * import { createLogger } from '@kurumi/log';
 * import { parseLogLevel } from '@kurumi/log';
 *
 * const logger = createLogger({ level: parseLogLevel('debug') ?? 'info' });
 * logger.child({ service: 'api' }).info('started', { port: 3000 });
 * await logger.close();
 * ```
 */
export function createLogger(options: LoggerOptions = {}): Logger {
  if (typeof options !== 'object' || options === null || Array.isArray(options)) {
    throw new LogDefinitionError('LoggerOptions 必须是普通对象');
  }
  const config = resolveLoggerConfig(options);
  const bindings = normalizeLogFields(options.bindings, config.normalization);
  return new DefaultLogger(config, bindings, {
    pending: new Set(),
    errors: [],
    lifecycle: { status: 'open' },
  });
}

class DefaultLogger implements Logger {
  readonly #config: LoggerConfig;
  readonly #bindings: ResolvedLogFields;
  readonly #state: LoggerState;

  constructor(config: LoggerConfig, bindings: ResolvedLogFields, state: LoggerState) {
    this.#config = config;
    this.#bindings = bindings;
    this.#state = state;
  }

  get level(): LogLevel {
    return this.#config.level;
  }

  isLevelEnabled(level: EmittedLogLevel): boolean {
    return isEmittedLogLevel(level) && isLevelEnabled(this.#config.level, level);
  }

  log(level: EmittedLogLevel, message: string, fields?: LogFieldsProvider): void {
    if (this.#state.lifecycle.status !== 'open') throw new LogClosedError();
    if (!isEmittedLogLevel(level)) {
      throw new LogDefinitionError(`不能写入非法日志级别: ${String(level)}`);
    }
    if (!isLevelEnabled(this.#config.level, level)) return;

    const timeMs = this.#config.clock();
    if (!Number.isFinite(timeMs)) {
      throw new LogDefinitionError(`clock 必须返回有限数值（实际值: ${String(timeMs)}）`);
    }

    const record = this.#createRecord(level, message, fields, timeMs);
    this.#dispatch(record);
  }

  trace(message: string, fields?: LogFieldsProvider): void {
    this.log(LogLevel.Trace, message, fields);
  }

  debug(message: string, fields?: LogFieldsProvider): void {
    this.log(LogLevel.Debug, message, fields);
  }

  info(message: string, fields?: LogFieldsProvider): void {
    this.log(LogLevel.Info, message, fields);
  }

  warn(message: string, fields?: LogFieldsProvider): void {
    this.log(LogLevel.Warn, message, fields);
  }

  error(message: string, fields?: LogFieldsProvider): void {
    this.log(LogLevel.Error, message, fields);
  }

  fatal(message: string, fields?: LogFieldsProvider): void {
    this.log(LogLevel.Fatal, message, fields);
  }

  child(bindings?: LogFields): Logger {
    const extra = normalizeLogFields(bindings, this.#config.normalization);
    return new DefaultLogger(this.#config, mergeLogFields(this.#bindings, extra), this.#state);
  }

  withLevel(level: LogLevel): Logger {
    if (!isLogLevel(level)) {
      throw new LogDefinitionError(`非法日志级别: ${String(level)}`);
    }
    return new DefaultLogger({ ...this.#config, level }, this.#bindings, this.#state);
  }

  async flush(): Promise<void> {
    const lifecycle = this.#state.lifecycle;
    if (lifecycle.status !== 'open') {
      await lifecycle.promise;
      return;
    }
    await this.#flushInternal();
  }

  close(): Promise<void> {
    const lifecycle = this.#state.lifecycle;
    if (lifecycle.status !== 'open') return lifecycle.promise;

    // 先进入 closing 再启动关闭流程，确保 sink.flush 期间的同步 log() 也被拒绝。
    let resolveClose = (): void => undefined;
    let rejectClose = (_reason?: unknown): void => undefined;
    const promise = new Promise<void>((resolve, reject) => {
      resolveClose = resolve;
      rejectClose = reject;
    });
    this.#state.lifecycle = { status: 'closing', promise };
    void this.#closeInternal().then(resolveClose, rejectClose);
    return promise;
  }

  async #closeInternal(): Promise<void> {
    const failures: LogSinkError[] = [];
    try {
      try {
        await this.#flushInternal();
      } catch (cause) {
        appendAggregateFailures(failures, cause);
      }

      // 按 sinks 声明顺序关闭；契约要求该顺序，不能并行关闭。
      for (const sink of this.#config.sinks) {
        try {
          await sink.close?.();
        } catch (cause) {
          failures.push(toSinkError(sink, '关闭失败', cause));
        }
      }
      if (failures.length > 0) throw new AggregateError(failures, '日志 sink 关闭失败');
    } finally {
      const current = this.#state.lifecycle;
      if (current.status === 'closing') {
        this.#state.lifecycle = { status: 'closed', promise: current.promise };
      }
    }
  }

  async #flushInternal(): Promise<void> {
    const failures: LogSinkError[] = [];
    // 不能在单次快照后停止：等待期间仍可能有新的异步写入加入 pending。
    for (;;) {
      while (this.#state.pending.size > 0) {
        await Promise.all(this.#state.pending);
      }

      for (const error of this.#state.errors.splice(0)) addFailure(failures, error);

      const flushResults = await Promise.allSettled(
        this.#config.sinks.map(async (sink) => {
          await sink.flush?.();
        }),
      );
      for (let index = 0; index < flushResults.length; index += 1) {
        const result = flushResults[index];
        const sink = this.#config.sinks[index];
        if (result === undefined || sink === undefined || result.status === 'fulfilled') continue;
        addFailure(failures, toSinkError(sink, '刷新失败', result.reason));
      }

      if (this.#state.pending.size === 0) break;
    }
    if (failures.length > 0) throw new AggregateError(failures, '日志 sink 刷新失败');
  }

  #createRecord(
    level: EmittedLogLevel,
    message: string,
    fields: LogFieldsProvider | undefined,
    timeMs: number,
  ): LogRecord {
    const resolvedFields = resolveFields(fields, this.#config.normalization);
    const record: LogRecord = {
      timeMs,
      level,
      message: truncateMessage(
        typeof message === 'string' ? message : String(message),
        this.#config.maxMessageLength,
      ),
      bindings: this.#bindings,
      ...(resolvedFields === undefined ? {} : { fields: resolvedFields }),
    };
    return deepFreeze(record);
  }

  #dispatch(record: LogRecord): void {
    const failures: LogSinkError[] = [];
    for (const sink of this.#config.sinks) {
      let result: void | Promise<void>;
      try {
        result = sink.write(record);
      } catch (cause) {
        this.#handleSyncFailure(sink, record, cause, failures);
        continue;
      }
      try {
        if (isPromiseLike(result)) this.#trackAsyncWrite(sink, record, result);
      } catch (cause) {
        this.#handleSyncFailure(sink, record, cause, failures);
      }
    }
    if (failures.length > 0) throw aggregateFailures(failures, '日志 sink 写入失败');
  }

  #handleSyncFailure(
    sink: LogSink,
    record: LogRecord,
    cause: unknown,
    failures: LogSinkError[],
  ): void {
    const error = toSinkError(sink, describeCause(cause), cause);
    const handler = this.#config.onSinkError;
    if (handler === undefined) {
      failures.push(error);
      return;
    }
    try {
      const handled = handler({ error, record, sink });
      if (isPromiseLike(handled)) this.#trackHandler(sink, handled);
    } catch (handlerCause) {
      failures.push(toSinkError(sink, 'onSinkError 抛出异常', handlerCause));
    }
  }

  #trackAsyncWrite(sink: LogSink, record: LogRecord, result: PromiseLike<void>): void {
    const task = (async () => {
      try {
        await result;
      } catch (cause) {
        this.#handleAsyncFailure(sink, record, cause);
      }
    })();
    this.#state.pending.add(task);
    void (async () => {
      try {
        await task;
      } finally {
        this.#state.pending.delete(task);
      }
    })();
  }

  #handleAsyncFailure(sink: LogSink, record: LogRecord, cause: unknown): void {
    const error = toSinkError(sink, describeCause(cause), cause);
    const handler = this.#config.onSinkError;
    if (handler === undefined) {
      this.#state.errors.push(error);
      return;
    }
    try {
      const handled = handler({ error, record, sink });
      if (isPromiseLike(handled)) this.#trackHandler(sink, handled);
    } catch (handlerCause) {
      this.#state.errors.push(toSinkError(sink, 'onSinkError 抛出异常', handlerCause));
    }
  }

  #trackHandler(sink: LogSink, handled: PromiseLike<void>): void {
    const task = (async () => {
      try {
        await handled;
      } catch (cause) {
        this.#state.errors.push(toSinkError(sink, 'onSinkError 抛出异常', cause));
      }
    })();
    this.#state.pending.add(task);
    void (async () => {
      try {
        await task;
      } finally {
        this.#state.pending.delete(task);
      }
    })();
  }
}

// 保留 LoggerLifecycle 类型在声明文件中可见，供下游实现自定义生命周期时复用。
export type { LoggerLifecycle };
