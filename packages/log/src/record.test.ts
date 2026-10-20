/**
 * @file 日志字段归一化单元测试。
 * @description 覆盖特殊值、错误、循环引用、危险键、非对象字段与合并。
 * @module log/record.test
 */

import { describe, expect, it } from 'vitest';

import {
  EMPTY_LOG_FIELDS,
  mergeLogFields,
  normalizeLogFields,
  type RecordNormalizationOptions,
} from './record.js';

const OPTIONS: RecordNormalizationOptions = {
  maxDepth: 8,
  maxStringLength: 8_192,
  maxEntries: 1_000,
  shouldIncludeStack: false,
};

describe('normalizeLogFields', () => {
  it('归一化普通字段并忽略 undefined', () => {
    const fields = normalizeLogFields(
      { text: 'x', count: 1, empty: null, missing: undefined },
      OPTIONS,
    );
    expect(fields).toEqual({ text: 'x', count: 1, empty: null });
    expect(Object.isFrozen(fields)).toBe(true);
  });

  it('空值返回共享冻结实例', () => {
    expect(normalizeLogFields(undefined, OPTIONS)).toBe(EMPTY_LOG_FIELDS);
    expect(normalizeLogFields(null, OPTIONS)).toBe(EMPTY_LOG_FIELDS);
    expect(Object.isFrozen(EMPTY_LOG_FIELDS)).toBe(true);
  });

  it('丢弃危险键并保留其他字段', () => {
    const value = JSON.parse('{"__proto__":{"polluted":true},"ok":1}') as Record<string, unknown>;
    expect(normalizeLogFields(value, OPTIONS)).toEqual({ ok: 1 });
    expect(({} as Record<string, unknown>)['polluted']).toBeUndefined();
  });

  it('非普通对象包装为 value 字段', () => {
    expect(normalizeLogFields([1, 2], OPTIONS)).toEqual({ value: [1, 2] });
    expect(normalizeLogFields(new Set([1]), OPTIONS)).toEqual({ value: [1] });
  });

  it('序列化 Error 字段并保留 cause', () => {
    const error = new Error('boom', { cause: new Error('root') });
    const fields = normalizeLogFields({ error }, OPTIONS);
    expect(fields['error']).toMatchObject({
      name: 'Error',
      message: 'boom',
      cause: { name: 'Error', message: 'root' },
    });
    expect((fields['error'] as { stack?: string }).stack).toBeUndefined();
  });

  it('循环引用不会导致栈溢出', () => {
    const cyclic: Record<string, unknown> = {};
    cyclic['self'] = cyclic;
    expect(normalizeLogFields(cyclic, OPTIONS)).toEqual({
      self: { self: '[Circular]' },
    });
  });

  it('属性 getter 抛错时标记为 [Unreadable]', () => {
    const value: Record<string, unknown> = {};
    Object.defineProperty(value, 'broken', {
      enumerable: true,
      get: () => {
        throw new Error('getter boom');
      },
    });
    expect(normalizeLogFields(value, OPTIONS)).toEqual({ broken: '[Unreadable]' });
  });
});

describe('mergeLogFields', () => {
  it('后者覆盖前者且不修改入参', () => {
    const base = normalizeLogFields({ service: 'api', region: 'cn' }, OPTIONS);
    const override = normalizeLogFields({ region: 'us' }, OPTIONS);
    const merged = mergeLogFields(base, override);
    expect(merged).toEqual({ service: 'api', region: 'us' });
    expect(base).toEqual({ service: 'api', region: 'cn' });
    expect(Object.isFrozen(merged)).toBe(true);
  });
});
