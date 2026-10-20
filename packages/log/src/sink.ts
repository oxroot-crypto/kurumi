/**
 * @file 日志 sink 公共契约。
 * @description sink 负责输出，不负责级别过滤或字段归一化。
 * @module log/sink
 */

import type { LogRecord } from './record.js';

/** 日志输出目标。 */
export interface LogSink {
  /** sink 名称，用于错误定位；缺省显示为 `<anonymous>`。 */
  readonly name?: string;
  /**
   * 写入一条记录。
   *
   * @param record - 已冻结的日志记录
   * @returns 同步写入时返回 void；异步写入时返回完成 Promise
   */
  write(record: LogRecord): void | Promise<void>;
  /**
   * 刷新内部缓冲；异步实现必须等待在飞行写入。
   *
   * @returns 刷新完成时 resolve
   */
  flush?(): void | Promise<void>;
  /**
   * 释放句柄；实现必须幂等。
   *
   * @returns 释放完成时 resolve
   */
  close?(): void | Promise<void>;
}
