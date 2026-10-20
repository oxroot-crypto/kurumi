/**
 * @file Logger 基础行为测试。
 * @description 覆盖级别过滤、记录形状、bindings、字段归一化与选项校验。
 * @module log/logger.test
 */

import { describe, expect, it, vi } from 'vitest';

import { LogDefinitionError } from './errors.js';
import { LogLevel } from './level.js';
import { createLogger, type LoggerOptions } from './logger.js';
import type { LogFields } from './record.js';
import { createMemorySink } from './sink/memory.js';
import type { LogSink } from './sink.js';

describe('DefaultLogger', () => {
  it('默认级别为 info', () => {
    const sink = createMemorySink();
    const logger = createLogger({ sinks: [sink], clock: () => 10 });
    logger.debug('hidden');
    logger.info('shown');
    expect(sink.records()).toHaveLength(1);
    expect(sink.records()[0]?.message).toBe('shown');
  });

  it('默认 sink 输出到 console', () => {
    const spy = vi.spyOn(console, 'info').mockImplementation(() => undefined);
    const logger = createLogger();
    logger.info('default sink');
    expect(spy).toHaveBeenCalledTimes(1);
  });

  it('silent 阈值关闭全部写入且不调用时钟', () => {
    const sink = createMemorySink();
    let clockCalls = 0;
    const logger = createLogger({
      level: LogLevel.Silent,
      sinks: [sink],
      clock: () => {
        clockCalls += 1;
        return 0;
      },
    });
    logger.fatal('hidden');
    expect(sink.records()).toEqual([]);
    expect(clockCalls).toBe(0);
  });

  it('构建冻结且字段归一化的记录', () => {
    const sink = createMemorySink();
    const logger = createLogger({ sinks: [sink], clock: () => 123 });
    logger
      .child({ service: 'api', nested: { region: 'cn' } })
      .warn('hello', { port: 3000, error: new Error('boom') });
    const record = sink.records()[0];
    expect(record).toMatchObject({
      timeMs: 123,
      level: 'warn',
      message: 'hello',
      bindings: { service: 'api', nested: { region: 'cn' } },
      fields: { port: 3000 },
    });
    expect(record?.fields?.['error']).toMatchObject({ name: 'Error', message: 'boom' });
    expect(Object.isFrozen(record)).toBe(true);
    expect(Object.isFrozen(record?.bindings)).toBe(true);
    expect(Object.isFrozen(record?.fields)).toBe(true);
  });

  it('非字符串 message 防御式转换', () => {
    const sink = createMemorySink();
    const logger = createLogger({ sinks: [sink] });
    Reflect.apply(logger.info, logger, [123]);
    expect(sink.records()[0]?.message).toBe('123');
  });

  it('child 浅合并 bindings 且不修改父级', () => {
    const sink = createMemorySink();
    const root = createLogger({ sinks: [sink], bindings: { service: 'api', region: 'cn' } });
    const child = root.child({ service: 'worker', requestId: 'r1' });
    child.info('child');
    root.info('root');

    expect(sink.records()[0]?.bindings).toEqual({
      service: 'worker',
      region: 'cn',
      requestId: 'r1',
    });
    expect(sink.records()[1]?.bindings).toEqual({ service: 'api', region: 'cn' });
  });

  it('withLevel 仅替换阈值并保留 bindings 与 sink', () => {
    const sink = createMemorySink();
    const root = createLogger({ sinks: [sink], bindings: { service: 'api' } });
    const debug = root.withLevel(LogLevel.Debug);
    debug.debug('visible');
    root.debug('hidden');
    expect(sink.records()).toHaveLength(1);
    expect(sink.records()[0]?.bindings).toEqual({ service: 'api' });
    expect(root.level).toBe(LogLevel.Info);
    expect(debug.level).toBe(LogLevel.Debug);
  });

  it('延迟字段仅在级别启用后求值', () => {
    const sink = createMemorySink();
    const logger = createLogger({ level: LogLevel.Info, sinks: [sink] });
    let calls = 0;
    const fields = (): LogFields => {
      calls += 1;
      return { value: calls };
    };
    logger.debug('hidden', fields);
    expect(calls).toBe(0);
    logger.info('shown', fields);
    expect(calls).toBe(1);
    expect(sink.records()[0]?.fields).toEqual({ value: 1 });
  });

  it('按声明顺序写入全部 sink', () => {
    const first = createMemorySink();
    const second = createMemorySink();
    const logger = createLogger({ sinks: [first, second] });
    logger.info('fan-out');
    expect(first.records()).toHaveLength(1);
    expect(second.records()).toHaveLength(1);
    expect(first.records()[0]).toBe(second.records()[0]);
  });

  it('顶层字段也遵守 maxEntries，message 遵守 maxMessageLength', () => {
    const sink = createMemorySink();
    const logger = createLogger({
      sinks: [sink],
      maxEntries: 2,
      maxMessageLength: 5,
    });
    logger.info('abcdef', { a: 1, b: 2, c: 3, d: 4 });
    const record = sink.records()[0];
    expect(record?.message).toBe('abcde');
    expect(Object.keys(record?.fields ?? {})).toHaveLength(3);
    expect(record?.fields?.['[truncated]']).toBeDefined();
  });

  it('字段 provider 抛错时降级为可诊断字段而不是中断日志', () => {
    const sink = createMemorySink();
    const logger = createLogger({ sinks: [sink] });
    logger.info('x', () => {
      throw new Error('fields boom');
    });
    expect(sink.records()).toHaveLength(1);
    expect(sink.records()[0]?.fields?.['[fields_error]']).toBeDefined();
  });

  it('拒绝非法选项', () => {
    const invalidLevel: string = 'verbose';
    expect(() => createLogger({ level: invalidLevel as LogLevel })).toThrow(LogDefinitionError);
    expect(() => createLogger({ maxDepth: -1 })).toThrow(LogDefinitionError);
    expect(() => createLogger({ maxStringLength: 0 })).toThrow(LogDefinitionError);
    expect(() => createLogger({ maxEntries: 0 })).toThrow(LogDefinitionError);
    expect(() => createLogger({ maxMessageLength: 0 })).toThrow(LogDefinitionError);
    expect(() => createLogger({ sinks: [{} as LogSink] })).toThrow(LogDefinitionError);

    const invalidClock: LoggerOptions = JSON.parse('{"clock":1}');
    expect(() => createLogger(invalidClock)).toThrow(LogDefinitionError);
    const invalidHandler: LoggerOptions = JSON.parse('{"onSinkError":1}');
    expect(() => createLogger(invalidHandler)).toThrow(LogDefinitionError);
    const invalidSinks: LoggerOptions = JSON.parse('{"sinks":{}}');
    expect(() => createLogger(invalidSinks)).toThrow(LogDefinitionError);
    const invalidStack: LoggerOptions = JSON.parse('{"shouldIncludeErrorStack":"yes"}');
    expect(() => createLogger(invalidStack)).toThrow(LogDefinitionError);
    const invalidOptions: LoggerOptions = JSON.parse('null');
    expect(() => createLogger(invalidOptions)).toThrow(LogDefinitionError);
    const invalidArrayOptions: LoggerOptions = JSON.parse('[]');
    expect(() => createLogger(invalidArrayOptions)).toThrow(LogDefinitionError);
  });

  it('时钟返回非有限数值时拒绝写入', () => {
    const logger = createLogger({ sinks: [createMemorySink()], clock: () => Number.NaN });
    expect(() => logger.info('x')).toThrow(LogDefinitionError);
  });
});
