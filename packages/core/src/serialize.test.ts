/**
 * @file JSON 安全序列化与错误序列化单元测试。
 * @description 覆盖特殊值、循环引用、深度/长度/条目限制、危险键、错误链与非法选项。
 * @module core/serialize.test
 */

import { describe, expect, it } from 'vitest';

import { KurumiError } from './errors.js';
import { isErrorLike, serializeError, toJsonSafeValue } from './serialize.js';

describe('isErrorLike', () => {
  it('识别具备字符串 name/message 的对象', () => {
    expect(isErrorLike(new Error('boom'))).toBe(true);
    expect(isErrorLike({ name: 'DomainError', message: 'bad' })).toBe(true);
    expect(isErrorLike(null)).toBe(false);
    expect(isErrorLike({ name: 1, message: 'bad' })).toBe(false);
    expect(isErrorLike('error')).toBe(false);
  });
});

describe('toJsonSafeValue', () => {
  it('保留 JSON 原始值并转换 undefined', () => {
    expect(
      toJsonSafeValue({
        text: 'kurumi',
        count: 1,
        enabled: true,
        empty: null,
        missing: undefined,
      }),
    ).toEqual({
      text: 'kurumi',
      count: 1,
      enabled: true,
      empty: null,
      missing: null,
    });
  });

  it('将非有限数值、BigInt、Symbol 与函数转换为稳定字符串', () => {
    expect(toJsonSafeValue(Number.NaN)).toBe('NaN');
    expect(toJsonSafeValue(Number.POSITIVE_INFINITY)).toBe('Infinity');
    expect(toJsonSafeValue(Number.NEGATIVE_INFINITY)).toBe('-Infinity');
    expect(toJsonSafeValue(12n)).toBe('12n');
    expect(toJsonSafeValue(Symbol('token'))).toBe('Symbol(token)');
    expect(toJsonSafeValue(function named() {})).toBe('[Function named]');
    expect(toJsonSafeValue(() => undefined)).toBe('[Function anonymous]');
  });

  it('转换 Date、RegExp、Map 与 Set', () => {
    const value = {
      date: new Date(0),
      invalidDate: new Date(Number.NaN),
      pattern: /ab+c/gi,
      map: new Map<unknown, unknown>([['a', 1]]),
      set: new Set([1, 2]),
    };
    expect(toJsonSafeValue(value)).toEqual({
      date: '1970-01-01T00:00:00.000Z',
      invalidDate: 'Invalid Date',
      pattern: '/ab+c/gi',
      map: [['a', 1]],
      set: [1, 2],
    });
  });

  it('循环引用输出 [Circular] 且重复引用不误判', () => {
    const shared = { value: 1 };
    const cyclic: Record<string, unknown> = { first: shared, second: shared };
    cyclic['self'] = cyclic;
    expect(toJsonSafeValue(cyclic)).toEqual({
      first: { value: 1 },
      second: { value: 1 },
      self: '[Circular]',
    });
  });

  it('超过深度上限输出 [MaxDepth]', () => {
    expect(toJsonSafeValue({ nested: { value: 1 } }, { maxDepth: 0 })).toEqual({
      nested: '[MaxDepth]',
    });
    expect(toJsonSafeValue({ nested: { value: 1 } }, { maxDepth: 1 })).toEqual({
      nested: { value: '[MaxDepth]' },
    });
  });

  it('超过字符串、条目上限时截断并标记', () => {
    expect(toJsonSafeValue('abcdef', { maxStringLength: 3 })).toBe('abc');
    expect(toJsonSafeValue('abcdef', { maxStringLength: 20 })).toBe('abcdef');
    expect(toJsonSafeValue('a'.repeat(30), { maxStringLength: 20 })).toHaveLength(20);
    expect(toJsonSafeValue('a'.repeat(30), { maxStringLength: 20 })).toContain('…[truncated]');
    expect(toJsonSafeValue([1, 2, 3], { maxEntries: 2 })).toEqual([
      1,
      2,
      '[Truncated: 1 more items]',
    ]);
    expect(toJsonSafeValue({ a: 1, b: 2, c: 3 }, { maxEntries: 2 })).toEqual({
      a: 1,
      b: 2,
      '[truncated]': '[1 more keys]',
    });
  });

  it('丢弃危险键且不污染 Object.prototype', () => {
    const value = JSON.parse('{"__proto__":{"polluted":true},"ok":1}') as Record<string, unknown>;
    expect(toJsonSafeValue(value)).toEqual({ ok: 1 });
    expect(({} as Record<string, unknown>)['polluted']).toBeUndefined();
  });

  it('属性读取异常时标记为 [Unreadable]', () => {
    const value: Record<string, unknown> = {};
    Object.defineProperty(value, 'broken', {
      enumerable: true,
      get: () => {
        throw new Error('getter boom');
      },
    });
    expect(toJsonSafeValue(value)).toEqual({ broken: '[Unreadable]' });
  });

  it('恶意 Proxy 抛错时退化为 [Unserializable]', () => {
    const proxy = new Proxy(
      {},
      {
        getPrototypeOf: () => {
          throw new Error('proxy boom');
        },
      },
    );
    expect(toJsonSafeValue(proxy)).toBe('[Unserializable]');
  });

  it('不修改输入对象', () => {
    const value = { nested: { count: 1 } };
    const result = toJsonSafeValue(value);
    expect(result).toEqual(value);
    expect(result).not.toBe(value);
    expect(value.nested.count).toBe(1);
  });

  it('对符号/BigInt/函数名与对象键应用字符串上限', () => {
    const long = 'x'.repeat(100);
    expect(toJsonSafeValue(Symbol(long), { maxStringLength: 30 })).toContain('[truncated]');
    expect(toJsonSafeValue(10n ** 100n, { maxStringLength: 30 })).toContain('[truncated]');
    expect(toJsonSafeValue(function veryLongFunctionName() {}, { maxStringLength: 30 })).toContain(
      '[truncated]',
    );
    const result = toJsonSafeValue({ [long]: 1 }, { maxStringLength: 30 }) as Record<
      string,
      unknown
    >;
    expect(Object.keys(result)).toHaveLength(1);
    expect(Object.keys(result)[0]).toContain('[truncated]');
  });

  it('拒绝非法选项', () => {
    expect(() => toJsonSafeValue({}, { maxDepth: -1 })).toThrow(KurumiError);
    expect(() => toJsonSafeValue({}, { maxDepth: 1.5 })).toThrow(KurumiError);
    expect(() => toJsonSafeValue('x', { maxStringLength: 0 })).toThrow(KurumiError);
    expect(() => toJsonSafeValue([], { maxEntries: 0 })).toThrow(KurumiError);

    const error = catchError(() => toJsonSafeValue({}, { maxDepth: -1 }));
    expect(error).toBeInstanceOf(KurumiError);
    expect((error as KurumiError).code).toBe('invalid_options');
  });
});

describe('serializeError', () => {
  it('保留 name、message、stack、code 与 cause', () => {
    const error = Object.assign(new Error('boom', { cause: new Error('root') }), {
      code: 'E_BOOM',
      detail: { retry: true },
    });
    const serialized = serializeError(error);
    expect(serialized.name).toBe('Error');
    expect(serialized.message).toBe('boom');
    expect(serialized.code).toBe('E_BOOM');
    expect(serialized['detail']).toEqual({ retry: true });
    expect(serialized.stack).toContain('Error: boom');
    expect(serialized.cause).toMatchObject({ name: 'Error', message: 'root' });
  });

  it('shouldIncludeStack=false 时不输出调用栈', () => {
    const serialized = serializeError(new Error('quiet'), { shouldIncludeStack: false });
    expect(serialized.stack).toBeUndefined();
  });

  it('循环 cause 输出 [Circular]', () => {
    const error = new Error('loop') as Error & { cause?: unknown };
    error.cause = error;
    expect(serializeError(error).cause).toBe('[Circular]');
  });

  it('超过深度上限的 cause 输出 [MaxDepth]', () => {
    const error = new Error('outer', { cause: new Error('inner') });
    expect(serializeError(error, { maxDepth: 0 }).cause).toBe('[MaxDepth]');
  });

  it('序列化 AggregateError 的子错误', () => {
    const error = new AggregateError([new Error('first'), 'second'], 'many');
    const serialized = serializeError(error);
    expect(serialized.name).toBe('AggregateError');
    expect(serialized.message).toBe('many');
    expect(serialized.errors).toEqual([
      expect.objectContaining({ name: 'Error', message: 'first' }),
      'second',
    ]);
  });

  it('AggregateError 包含自身时标记为 [Circular]', () => {
    const error = new AggregateError([], 'loop');
    error.errors.push(error);
    expect(serializeError(error).errors).toEqual(['[Circular]']);
  });

  it('非 Error 字符串也遵守长度上限', () => {
    const serialized = serializeError('x'.repeat(100), { maxStringLength: 10 });
    expect(serialized.name).toBe('NonError');
    expect(serialized.message).toHaveLength(10);
  });

  it('非 Error 输入包装为 NonError', () => {
    expect(serializeError('oops')).toEqual({ name: 'NonError', message: 'oops' });
    expect(serializeError(undefined)).toEqual({ name: 'NonError', message: 'undefined' });
    expect(serializeError({ code: 'E' })).toEqual({
      name: 'NonError',
      message: '{"code":"E"}',
    });
  });

  it('Error 序列化结果也是冻结对象', () => {
    expect(Object.isFrozen(serializeError(new Error('x')))).toBe(true);
    expect(Object.isFrozen(serializeError('x'))).toBe(true);
  });

  it('拒绝非法选项', () => {
    expect(() => serializeError(new Error('x'), { maxEntries: -2 })).toThrow(KurumiError);
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
