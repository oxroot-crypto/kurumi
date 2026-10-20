/**
 * @file core/validation 单元测试。
 * @description 覆盖通用校验工具的类型、边界、集合与自定义错误工厂。
 * @module core/validation.test
 */

import { describe, expect, it } from 'vitest';

import { KurumiError } from './errors.js';
import {
  CoreValidationError,
  assertArrayMaxLength,
  assertBoolean,
  assertFiniteNumber,
  assertNonEmptyString,
  assertOneOf,
  assertPattern,
  assertPlainObject,
  assertPositiveSafeInteger,
  assertRange,
  assertSafeInteger,
  assertString,
  assertTimestampMs,
  describeValue,
} from './validation.js';

class DomainError extends KurumiError {
  constructor(message: string) {
    super(message, { code: 'domain_error' });
  }
}

const domainError = (message: string) => new DomainError(message);

describe('字符串校验', () => {
  it('接受合法字符串并拒绝空白', () => {
    expect(() => assertString('x', 'field')).not.toThrow();
    expect(() => assertString(1, 'field')).toThrow(CoreValidationError);
    expect(() => assertNonEmptyString('x', 'field')).not.toThrow();
    expect(() => assertNonEmptyString('  ', 'field')).toThrow(CoreValidationError);
    expect(() => assertPattern('abc', /^a/, 'field')).not.toThrow();
    expect(() => assertPattern('abc', /^z/, 'field')).toThrow(CoreValidationError);
    const globalPattern = /^a/g;
    expect(() => assertPattern('abc', globalPattern, 'field')).not.toThrow();
    expect(() => assertPattern('abc', globalPattern, 'field')).not.toThrow();
    expect(globalPattern.lastIndex).toBe(0);
  });
});

describe('数值与时间校验', () => {
  it('安全整数、正安全整数、时间戳与有限数', () => {
    expect(() => assertSafeInteger(1, 'n')).not.toThrow();
    expect(() => assertSafeInteger(-1, 'n')).toThrow(CoreValidationError);
    expect(() => assertSafeInteger(1.5, 'n')).toThrow(CoreValidationError);
    expect(() => assertPositiveSafeInteger(1, 'n')).not.toThrow();
    expect(() => assertPositiveSafeInteger(0, 'n')).toThrow(CoreValidationError);
    expect(() => assertTimestampMs(0, 't')).not.toThrow();
    expect(() => assertFiniteNumber(1.5, 'n')).not.toThrow();
    expect(() => assertFiniteNumber(Number.NaN, 'n')).toThrow(CoreValidationError);
    expect(() => assertRange(5, 'n', 0, 10)).not.toThrow();
    expect(() => assertRange(11, 'n', 0, 10)).toThrow(CoreValidationError);
    expect(() => assertRange(Number.NaN, 'n', 0, 10)).toThrow(CoreValidationError);
    expect(() => assertRange(Number.POSITIVE_INFINITY, 'n', 0, 10)).toThrow(CoreValidationError);
    expect(() => assertRange(5, 'n', 10, 0)).toThrow(CoreValidationError);
  });
});

describe('集合与对象校验', () => {
  it('布尔、枚举、普通对象与数组长度', () => {
    expect(() => assertBoolean(true, 'b')).not.toThrow();
    expect(() => assertBoolean('true', 'b')).toThrow(CoreValidationError);
    expect(() => assertOneOf('a', ['a', 'b'], 'v')).not.toThrow();
    expect(() => assertOneOf('c', ['a', 'b'], 'v')).toThrow(CoreValidationError);
    expect(() => assertPlainObject({}, 'o')).not.toThrow();
    expect(() => assertPlainObject([], 'o')).toThrow(CoreValidationError);
    expect(() => assertArrayMaxLength([1, 2], 'list', 2)).not.toThrow();
    expect(() => assertArrayMaxLength([1, 2, 3], 'list', 2)).toThrow(CoreValidationError);
  });
});

describe('自定义错误工厂与描述', () => {
  it('可注入领域错误', () => {
    expect(() => assertString(1, 'field', domainError)).toThrow(DomainError);
    expect(() => assertPlainObject(null, 'field', domainError)).toThrow(DomainError);
  });

  it('describeValue 截断超长文本且不因不可序列化值抛错', () => {
    expect(describeValue(1)).toBe('1');
    expect(describeValue('x'.repeat(100))).toHaveLength(81);
    expect(describeValue(Object.create(null))).toBe('[Unserializable]');
    expect(
      describeValue({
        toString(): string {
          throw new Error('boom');
        },
      }),
    ).toBe('[Unserializable]');
  });
});
