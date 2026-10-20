/**
 * @file console sink 单元测试。
 * @description 覆盖 pretty/JSON 格式、级别路由、默认 console 与非法格式。
 * @module log/sink/console.test
 */

import { describe, expect, it } from 'vitest';

import { LogDefinitionError, LogSinkError } from '../errors.js';
import type { LogFormat } from '../format.js';
import { LogLevel } from '../level.js';
import { createLogger } from '../logger.js';
import { createConsoleSink, type ConsoleLike } from './console.js';

interface ConsoleCapture {
  readonly console: ConsoleLike;
  readonly lines: string[];
  readonly methods: string[];
}

function captureConsole(): ConsoleCapture {
  const lines: string[] = [];
  const methods: string[] = [];
  const record =
    (method: string) =>
    (...args: readonly unknown[]) => {
      methods.push(method);
      lines.push(String(args[0]));
    };
  return {
    console: {
      debug: record('debug'),
      info: record('info'),
      warn: record('warn'),
      error: record('error'),
      log: record('log'),
    },
    lines,
    methods,
  };
}

describe('createConsoleSink', () => {
  it('pretty 格式包含时间、级别、消息与结构化字段', () => {
    const capture = captureConsole();
    const logger = createLogger({
      level: LogLevel.Debug,
      sinks: [createConsoleSink({ console: capture.console })],
      clock: () => 0,
      bindings: { service: 'api' },
    });
    logger.info('started', { port: 3000 });
    expect(capture.lines).toEqual([
      '1970-01-01T00:00:00.000Z INFO started bindings={"service":"api"} fields={"port":3000}',
    ]);
  });

  it('JSON 格式输出单行合法 JSON', () => {
    const capture = captureConsole();
    const logger = createLogger({
      sinks: [createConsoleSink({ console: capture.console, format: 'json' })],
      clock: () => 0,
      bindings: { service: 'api' },
    });
    logger.warn('slow', { latencyMs: 120 });
    const line = capture.lines[0];
    expect(line).toBeDefined();
    expect(JSON.parse(line ?? '')).toEqual({
      timeMs: 0,
      level: 'warn',
      message: 'slow',
      bindings: { service: 'api' },
      fields: { latencyMs: 120 },
    });
  });

  it('按级别路由到对应 console 方法', () => {
    const capture = captureConsole();
    const sink = createConsoleSink({ console: capture.console, format: 'json' });
    const logger = createLogger({ level: LogLevel.Trace, sinks: [sink] });
    logger.trace('t');
    logger.debug('d');
    logger.info('i');
    logger.warn('w');
    logger.error('e');
    logger.fatal('f');
    expect(capture.methods).toEqual(['debug', 'debug', 'info', 'warn', 'error', 'error']);
  });

  it('console 缺少对应方法时回退到 log', () => {
    const lines: string[] = [];
    const logger = createLogger({
      sinks: [
        createConsoleSink({
          console: {
            log: (...args: readonly unknown[]) => {
              lines.push(String(args[0]));
            },
          },
        }),
      ],
    });
    logger.info('fallback');
    expect(lines).toHaveLength(1);
  });

  it('console 不可用时由 Logger 包装为 LogSinkError', () => {
    const logger = createLogger({ sinks: [createConsoleSink({ console: {} })] });
    expect(() => logger.info('x')).toThrow(LogSinkError);
  });

  it('拒绝未知格式', () => {
    const invalidFormat: string = 'xml';
    expect(() => createConsoleSink({ format: invalidFormat as LogFormat })).toThrow(
      LogDefinitionError,
    );
  });
});
