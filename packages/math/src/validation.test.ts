/**
 * @file math/validation 单元测试。
 * @description 覆盖 Decimal 断言、默认错误与自定义领域错误工厂。
 * @module math/validation.test
 */

import { describe, expect, it } from 'vitest';

import { KurumiError, type ValidationErrorFactory } from '@kurumi/core';

import { Decimal } from './decimal.js';
import { MathDefinitionError } from './errors.js';
import {
  assertDecimal,
  assertNonNegativeDecimal,
  assertPositiveDecimal,
  mathValidationError,
} from './validation.js';

class DomainError extends KurumiError {
  constructor(message: string) {
    super(message, { code: 'domain_error' });
  }
}

describe('math validation', () => {
  it('接受/拒绝 Decimal 与正负约束', () => {
    expect(() => assertDecimal(Decimal.ONE, 'value')).not.toThrow();
    expect(() => assertDecimal('1', 'value')).toThrow(MathDefinitionError);
    expect(() => assertPositiveDecimal(Decimal.ONE, 'value')).not.toThrow();
    expect(() => assertPositiveDecimal(Decimal.ZERO, 'value')).toThrow(MathDefinitionError);
    expect(() => assertNonNegativeDecimal(Decimal.ZERO, 'value')).not.toThrow();
    expect(() => assertNonNegativeDecimal(Decimal.parse('-1'), 'value')).toThrow(
      MathDefinitionError,
    );
  });

  it('支持自定义错误工厂', () => {
    const domainError: ValidationErrorFactory = (message) => new DomainError(message);
    expect(() => assertDecimal('1', 'value', domainError)).toThrow(DomainError);
    expect(mathValidationError('boom')).toBeInstanceOf(MathDefinitionError);
  });
});
