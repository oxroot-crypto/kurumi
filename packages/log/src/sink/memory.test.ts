/**
 * @file 内存 sink 测试。
 * @description 覆盖记录副本、清空与冻结记录引用。
 * @module log/sink/memory.test
 */

import { describe, expect, it } from 'vitest';

import type { LogRecord } from '../record.js';
import { createMemorySink } from './memory.js';

const RECORD: LogRecord = {
  timeMs: 0,
  level: 'info',
  message: 'hello',
  bindings: {},
};

describe('createMemorySink', () => {
  it('records 返回副本，clear 清空内部集合', () => {
    const sink = createMemorySink();
    sink.write(RECORD);
    const first = sink.records();
    expect(first).toEqual([RECORD]);
    expect(first).not.toBe(sink.records());

    sink.clear();
    expect(sink.records()).toEqual([]);
  });
});
