/**
 * @file 日志级别单元测试。
 * @description 覆盖顺序、过滤阈值、解析、标签与非法输入。
 * @module log/level.test
 */

import { describe, expect, it } from 'vitest';

import {
  ALL_LOG_LEVELS,
  EMITTED_LOG_LEVELS,
  LogLevel,
  formatLogLevel,
  isEmittedLogLevel,
  isLevelEnabled,
  isLogLevel,
  parseLogLevel,
} from './level.js';

describe('日志级别', () => {
  it('按严重程度从低到高排列', () => {
    expect(EMITTED_LOG_LEVELS).toEqual(['trace', 'debug', 'info', 'warn', 'error', 'fatal']);
    expect(ALL_LOG_LEVELS).toEqual([...EMITTED_LOG_LEVELS, 'silent']);
    expect(Object.isFrozen(LogLevel)).toBe(true);
    expect(Object.isFrozen(EMITTED_LOG_LEVELS)).toBe(true);
    expect(Object.isFrozen(ALL_LOG_LEVELS)).toBe(true);
  });

  it('按阈值过滤', () => {
    expect(isLevelEnabled(LogLevel.Info, LogLevel.Debug)).toBe(false);
    expect(isLevelEnabled(LogLevel.Info, LogLevel.Info)).toBe(true);
    expect(isLevelEnabled(LogLevel.Info, LogLevel.Fatal)).toBe(true);
    expect(isLevelEnabled(LogLevel.Trace, LogLevel.Trace)).toBe(true);
    expect(isLevelEnabled(LogLevel.Silent, LogLevel.Fatal)).toBe(false);
  });

  it('解析大小写不敏感并允许空白', () => {
    expect(parseLogLevel('WARN')).toBe('warn');
    expect(parseLogLevel('  debug  ')).toBe('debug');
    expect(parseLogLevel('silent')).toBe('silent');
    expect(parseLogLevel('verbose')).toBeUndefined();
    expect(parseLogLevel('')).toBeUndefined();
    expect(parseLogLevel(1)).toBeUndefined();
    expect(parseLogLevel(null)).toBeUndefined();
  });

  it('区分全部级别与可写级别', () => {
    expect(isLogLevel('silent')).toBe(true);
    expect(isLogLevel('verbose')).toBe(false);
    expect(isEmittedLogLevel('silent')).toBe(false);
    expect(isEmittedLogLevel('fatal')).toBe(true);
    expect(isEmittedLogLevel(undefined)).toBe(false);
  });

  it('生成大写标签', () => {
    expect(formatLogLevel(LogLevel.Trace)).toBe('TRACE');
    expect(formatLogLevel(LogLevel.Fatal)).toBe('FATAL');
  });
});
