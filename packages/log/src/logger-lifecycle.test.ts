/**
 * @file Logger flush/close 生命周期测试。
 * @description 覆盖在飞行写入、并发关闭、关闭幂等与 sink 关闭失败聚合。
 * @module log/logger-lifecycle.test
 */

import { describe, expect, it } from 'vitest';

import { LogClosedError } from './errors.js';
import { createLogger } from './logger.js';
import type { LogSink } from './sink.js';
import { createMemorySink } from './sink/memory.js';

describe('DefaultLogger lifecycle', () => {
  it('flush 等待在飞行异步写入', async () => {
    const gate = createGate();
    const sink: LogSink = {
      name: 'gated',
      async write() {
        await gate.promise;
      },
    };
    const logger = createLogger({ sinks: [sink] });
    logger.info('x');
    let flushed = false;
    const flushing = (async () => {
      await logger.flush();
      flushed = true;
    })();
    await Promise.resolve();
    expect(flushed).toBe(false);
    gate.open();
    await flushing;
    expect(flushed).toBe(true);
  });

  it('flush 等待 flush 开始后新加入的异步写入', async () => {
    const firstGate = createGate();
    const secondGate = createGate();
    const written: string[] = [];
    const sink: LogSink = {
      name: 'slow',
      async write(record) {
        await (record.message === 'first' ? firstGate.promise : secondGate.promise);
        written.push(record.message);
      },
    };
    const logger = createLogger({ sinks: [sink] });
    logger.info('first');
    const flushing = logger.flush();
    logger.info('second');
    let flushed = false;
    const observer = (async () => {
      await flushing;
      flushed = true;
    })();
    await Promise.resolve();
    firstGate.open();
    await Promise.resolve();
    await Promise.resolve();
    expect(flushed).toBe(false);
    expect(written).toEqual(['first']);

    secondGate.open();
    await flushing;
    await observer;
    expect(written).toEqual(['first', 'second']);
  });

  it('并发 close 复用同一关闭流程并等待 sink 释放', async () => {
    const gate = createGate();
    let closeCalls = 0;
    const sink: LogSink = {
      name: 'gated-close',
      write() {},
      async close() {
        closeCalls += 1;
        await gate.promise;
      },
    };
    const logger = createLogger({ sinks: [sink] });
    const first = logger.close();
    const second = logger.close();
    let secondSettled = false;
    const observer = (async () => {
      await second;
      secondSettled = true;
    })();
    await Promise.resolve();
    expect(secondSettled).toBe(false);

    gate.open();
    await first;
    await second;
    await observer;
    expect(closeCalls).toBe(1);
  });

  it('close 幂等、刷新 sink、关闭后拒绝写入', async () => {
    const sink = createMemorySink();
    const events: string[] = [];
    const tracked: LogSink = {
      name: 'tracked',
      write(record) {
        sink.write(record);
      },
      flush() {
        events.push('flush');
      },
      close() {
        events.push('close');
      },
    };
    const logger = createLogger({ sinks: [tracked] });
    logger.info('x');
    await logger.close();
    await logger.close();
    expect(events).toEqual(['flush', 'close']);
    expect(() => logger.info('after')).toThrow(LogClosedError);
    await expect(logger.flush()).resolves.toBeUndefined();
  });

  it('close 等待在飞行写入且关闭后立即拒绝新日志', async () => {
    const gate = createGate();
    const written: string[] = [];
    const sink: LogSink = {
      name: 'gated',
      async write(record) {
        await gate.promise;
        written.push(record.message);
      },
    };
    const logger = createLogger({ sinks: [sink] });
    logger.info('first');
    const closing = logger.close();
    expect(() => logger.info('second')).toThrow(LogClosedError);
    gate.open();
    await closing;
    expect(written).toEqual(['first']);
  });

  it('close 串行关闭 sink 并聚合关闭失败', async () => {
    const events: string[] = [];
    const first: LogSink = {
      name: 'first',
      write() {},
      close() {
        events.push('first');
        throw new Error('close boom');
      },
    };
    const second: LogSink = {
      name: 'second',
      write() {},
      close() {
        events.push('second');
      },
    };
    const logger = createLogger({ sinks: [first, second] });
    const error = await catchAsync(() => logger.close());
    expect(error).toBeInstanceOf(AggregateError);
    expect(events).toEqual(['first', 'second']);
  });
});

async function catchAsync(run: () => Promise<unknown>): Promise<unknown> {
  try {
    return await run();
  } catch (error) {
    return error;
  }
}

function createGate(): { readonly promise: Promise<void>; readonly open: () => void } {
  const { promise, resolve } = Promise.withResolvers<void>();
  return {
    promise,
    open: () => {
      resolve();
    },
  };
}
