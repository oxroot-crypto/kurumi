/**
 * @file 日志格式化单元测试。
 * @description 覆盖 pretty/json 输出与非法 format 的运行期拒绝。
 * @module log/format.test
 */

import { describe, expect, it } from 'vitest';

import { LogDefinitionError } from './errors.js';
import { formatLogRecord, type LogFormat } from './format.js';
import type { LogRecord } from './record.js';

const RECORD: LogRecord = {
  timeMs: 0,
  level: 'info',
  message: 'hello',
  bindings: { service: 'api' },
  fields: { port: 3000 },
};

describe('formatLogRecord', () => {
  it('输出 pretty 文本', () => {
    expect(formatLogRecord(RECORD)).toBe(
      '1970-01-01T00:00:00.000Z INFO hello bindings={"service":"api"} fields={"port":3000}',
    );
  });

  it('输出单行 JSON', () => {
    expect(JSON.parse(formatLogRecord(RECORD, 'json'))).toEqual(RECORD);
  });

  it('运行期拒绝未知 format', () => {
    expect(() => formatLogRecord(RECORD, 'xml' as LogFormat)).toThrow(LogDefinitionError);
  });

  it('非法时间输出 invalid-time', () => {
    expect(formatLogRecord({ ...RECORD, timeMs: Number.NaN })).toContain('invalid-time');
  });
});
