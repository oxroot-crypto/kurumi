/**
 * @file 内存 sink：用于测试、诊断与无输出 runtime。
 * @description 记录本身已冻结，访问器返回数组副本，避免外部修改内部集合。
 * @module log/sink/memory
 */

import type { LogRecord } from '../record.js';
import type { LogSink } from '../sink.js';

/** 内存 sink 的扩展接口。 */
export interface MemoryLogSink extends LogSink {
  /**
   * 返回按写入顺序排列的记录副本。
   *
   * @returns 记录副本，修改它不会影响内部集合
   */
  records(): readonly LogRecord[];
  /**
   * 清空已记录内容。
   */
  clear(): void;
}

/**
 * 创建内存 sink。
 *
 * @returns 可读取、可清空的 sink
 */
export function createMemorySink(): MemoryLogSink {
  const captured: LogRecord[] = [];
  return {
    name: 'memory',
    write(record: LogRecord): void {
      captured.push(record);
    },
    records(): readonly LogRecord[] {
      return [...captured];
    },
    clear(): void {
      captured.length = 0;
    },
  };
}
