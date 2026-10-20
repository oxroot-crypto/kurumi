/**
 * @file Logger sink 失败处理测试。
 * @description 覆盖同步/异步 sink 失败、onSinkError 包装与 flush 失败隔离。
 * @module log/logger-sink.test
 */

import { describe, expect, it } from 'vitest';

import { LogSinkError } from './errors.js';
import { createLogger, type LogSinkFailure } from './logger.js';
import { createMemorySink } from './sink/memory.js';
import type { LogSink } from './sink.js';

describe('DefaultLogger sink 失败处理', () => {
  it('同步 sink 失败仍尝试后续 sink，并以 LogSinkError 抛出', () => {
    const failing: LogSink = {
      name: 'broken',
      write() {
        throw new Error('write boom');
      },
    };
    const sink = createMemorySink();
    const logger = createLogger({ sinks: [failing, sink] });
    expect(() => logger.info('x')).toThrow(LogSinkError);
    expect(sink.records()).toHaveLength(1);
  });

  it('多个同步失败聚合为 AggregateError', () => {
    const failing = (name: string): LogSink => ({
      name,
      write() {
        throw new Error(`${name} boom`);
      },
    });
    const logger = createLogger({ sinks: [failing('a'), failing('b')] });
    const error = catchError(() => logger.info('x'));
    expect(error).toBeInstanceOf(AggregateError);
    expect((error as AggregateError).errors).toHaveLength(2);
  });

  it('onSinkError 接管同步失败后不抛出', () => {
    const failures: LogSinkFailure[] = [];
    const failing: LogSink = {
      name: 'broken',
      write() {
        throw new Error('write boom');
      },
    };
    const sink = createMemorySink();
    const logger = createLogger({
      sinks: [failing, sink],
      onSinkError: (failure) => {
        failures.push(failure);
      },
    });
    expect(() => logger.info('x')).not.toThrow();
    expect(failures).toHaveLength(1);
    expect(failures[0]?.error).toBeInstanceOf(LogSinkError);
    expect(failures[0]?.sink).toBe(failing);
    expect(sink.records()).toHaveLength(1);
  });

  it('onSinkError 同步抛错被包装为 LogSinkError', () => {
    const failing: LogSink = {
      name: 'broken',
      write() {
        throw new Error('write boom');
      },
    };
    const logger = createLogger({
      sinks: [failing],
      onSinkError: () => {
        throw new Error('handler boom');
      },
    });
    const error = catchError(() => logger.info('x'));
    expect(error).toBeInstanceOf(LogSinkError);
    expect((error as LogSinkError).message).toContain('onSinkError');
  });

  it('异步 sink 失败由 flush 显式抛出', async () => {
    const failing: LogSink = {
      name: 'async-broken',
      async write() {
        await Promise.resolve();
        throw new Error('async boom');
      },
    };
    const logger = createLogger({ sinks: [failing] });
    logger.info('x');
    const error = await catchAsync(() => logger.flush());
    expect(error).toBeInstanceOf(AggregateError);
    const [sinkError] = (error as AggregateError).errors;
    expect(sinkError).toBeInstanceOf(LogSinkError);
    expect((sinkError as LogSinkError).sinkName).toBe('async-broken');
  });

  it('onSinkError 接管异步失败后 flush 正常完成', async () => {
    const failures: LogSinkFailure[] = [];
    const failing: LogSink = {
      name: 'async-broken',
      async write() {
        throw new Error('async boom');
      },
    };
    const logger = createLogger({
      sinks: [failing],
      onSinkError: (failure) => {
        failures.push(failure);
      },
    });
    logger.info('x');
    await expect(logger.flush()).resolves.toBeUndefined();
    expect(failures).toHaveLength(1);
  });

  it('onSinkError 自身抛错被包装并在 flush 中暴露', async () => {
    const failing: LogSink = {
      name: 'async-broken',
      async write() {
        throw new Error('async boom');
      },
    };
    const logger = createLogger({
      sinks: [failing],
      onSinkError: () => {
        throw new Error('handler boom');
      },
    });
    logger.info('x');
    const error = await catchAsync(() => logger.flush());
    expect(error).toBeInstanceOf(AggregateError);
    const [sinkError] = (error as AggregateError).errors;
    expect(sinkError).toBeInstanceOf(LogSinkError);
    expect((sinkError as LogSinkError).message).toContain('onSinkError');
  });

  it('异步 onSinkError rejection 在 flush 中显式暴露', async () => {
    const failing: LogSink = {
      name: 'async-broken',
      async write() {
        throw new Error('write boom');
      },
    };
    const logger = createLogger({
      sinks: [failing],
      onSinkError: async () => {
        await Promise.resolve();
        throw new Error('handler boom');
      },
    });
    logger.info('x');
    const error = await catchAsync(() => logger.flush());
    expect(error).toBeInstanceOf(AggregateError);
    const [sinkError] = (error as AggregateError).errors;
    expect(sinkError).toBeInstanceOf(LogSinkError);
    expect((sinkError as LogSinkError).message).toContain('onSinkError');
  });

  it('一个 sink 的 flush 失败不会阻止其他 sink flush', async () => {
    const events: string[] = [];
    const failing: LogSink = {
      name: 'flush-broken',
      write() {},
      flush() {
        throw new Error('flush boom');
      },
    };
    const healthy: LogSink = {
      name: 'flush-ok',
      write() {},
      flush() {
        events.push('healthy');
      },
    };
    const logger = createLogger({ sinks: [failing, healthy] });
    const error = await catchAsync(() => logger.flush());
    expect(error).toBeInstanceOf(AggregateError);
    expect(events).toEqual(['healthy']);
  });
});

function catchError(run: () => unknown): unknown {
  try {
    run();
  } catch (error) {
    return error;
  }
  throw new Error('预期抛出错误，但调用成功');
}

async function catchAsync(run: () => Promise<unknown>): Promise<unknown> {
  try {
    return await run();
  } catch (error) {
    return error;
  }
}
