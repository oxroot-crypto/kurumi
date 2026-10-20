/**
 * @file console sink。
 * @description 只使用全局 console 的通用方法，不依赖 Node/浏览器专有 API。
 * @module log/sink/console
 */

import { LogDefinitionError } from '../errors.js';
import { formatLogRecord, type LogFormat } from '../format.js';
import { LogLevel, type EmittedLogLevel } from '../level.js';
import type { LogRecord } from '../record.js';
import type { LogSink } from '../sink.js';

/** 最小 console 接口；允许替换为测试替身。 */
export interface ConsoleLike {
  /** 调试输出。 */
  debug?(...args: readonly unknown[]): void;
  /** 普通信息输出。 */
  info?(...args: readonly unknown[]): void;
  /** 警告输出。 */
  warn?(...args: readonly unknown[]): void;
  /** 错误输出。 */
  error?(...args: readonly unknown[]): void;
  /** 兜底输出。 */
  log?(...args: readonly unknown[]): void;
}

/** console sink 选项。 */
export interface ConsoleSinkOptions {
  /** console 替身，缺省读取 `globalThis.console`。 */
  readonly console?: ConsoleLike;
  /** 输出格式，缺省 `pretty`。 */
  readonly format?: LogFormat;
}

/**
 * 创建 console sink。
 *
 * @param options - console 替身与输出格式
 * @returns 同步写入 console 的 sink
 * @throws {LogDefinitionError} 格式非法
 *
 * @example
 * ```ts
 * const sink = createConsoleSink({ format: 'json' });
 * ```
 */
export function createConsoleSink(options: ConsoleSinkOptions = {}): LogSink {
  if (typeof options !== 'object' || options === null || Array.isArray(options)) {
    throw new LogDefinitionError('ConsoleSinkOptions 必须是普通对象');
  }
  const format = options.format ?? 'pretty';
  if (format !== 'pretty' && format !== 'json') {
    throw new LogDefinitionError(`未知日志格式: ${String(format)}`);
  }
  const target = options.console ?? readGlobalConsole();

  return {
    name: 'console',
    write(record: LogRecord): void {
      const method = selectMethod(target, record.level);
      if (method === undefined) {
        throw new LogDefinitionError('当前 runtime 不提供可用的 console 输出方法');
      }
      method(formatLogRecord(record, format));
    },
  };
}

function selectMethod(
  target: ConsoleLike | undefined,
  level: EmittedLogLevel,
): ((...args: readonly unknown[]) => void) | undefined {
  if (target === undefined) return undefined;
  const candidates =
    level === LogLevel.Trace || level === LogLevel.Debug
      ? [target.debug, target.log]
      : level === LogLevel.Info
        ? [target.info, target.log]
        : level === LogLevel.Warn
          ? [target.warn, target.log]
          : [target.error, target.log];
  for (const candidate of candidates) {
    if (typeof candidate === 'function') return candidate.bind(target);
  }
  return undefined;
}

function readGlobalConsole(): ConsoleLike | undefined {
  try {
    const candidate = Reflect.get(globalThis, 'console');
    return isConsoleLike(candidate) ? candidate : undefined;
  } catch {
    return undefined;
  }
}

/** 判断值是否为可用的 console 形状。 */
function isConsoleLike(value: unknown): value is ConsoleLike {
  return typeof value === 'object' && value !== null;
}
