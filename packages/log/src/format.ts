/**
 * @file 日志记录文本格式化。
 * @description 提供跨 runtime 的 pretty / 单行 JSON 两种稳定格式。
 * @module log/format
 */

import { LogDefinitionError } from './errors.js';
import { formatLogLevel } from './level.js';
import type { LogRecord } from './record.js';

/** 内置输出格式。 */
export type LogFormat = 'pretty' | 'json';

/**
 * 将日志记录格式化为单行文本。
 *
 * @param record - 已归一化的日志记录
 * @param format - `pretty` 人类可读文本或 `json` 单行 JSON
 * @returns 不含尾随换行的字符串
 * @throws {LogDefinitionError} format 不是 pretty/json
 *
 * @example
 * ```ts
 * import { formatLogRecord } from '@kurumi/log';
 * const record = { timeMs: 0, level: 'info', message: 'hello', bindings: {} } as const;
 * formatLogRecord(record, 'json');
 * ```
 */
export function formatLogRecord(record: LogRecord, format: LogFormat = 'pretty'): string {
  if (format === 'json') return JSON.stringify(record);
  if (format !== 'pretty') throw new LogDefinitionError(`未知日志格式: ${String(format)}`);
  return formatPretty(record);
}

function formatPretty(record: LogRecord): string {
  const parts = [formatTime(record.timeMs), formatLogLevel(record.level), record.message];
  if (Object.keys(record.bindings).length > 0) {
    parts.push(`bindings=${JSON.stringify(record.bindings)}`);
  }
  if (record.fields !== undefined && Object.keys(record.fields).length > 0) {
    parts.push(`fields=${JSON.stringify(record.fields)}`);
  }
  return parts.join(' ');
}

function formatTime(timeMs: number): string {
  if (!Number.isFinite(timeMs)) return 'invalid-time';
  const date = new Date(timeMs);
  return Number.isNaN(date.getTime()) ? 'invalid-time' : date.toISOString();
}
