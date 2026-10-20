/**
 * @file Decimal 属性测试：代数不变量、解析往返与舍入误差界。
 * @description 覆盖 FC-DEC-INV-* 与 PBT-FIN-001。
 * @module math/decimal.properties.test
 */

import fc from 'fast-check';
import { describe, expect, it } from 'vitest';

import { Decimal, isDecimalMultipleOf, roundDecimalToStep } from './decimal.js';

const unitsArb = fc.bigInt({ min: -(10n ** 12n), max: 10n ** 12n });
const scaleArb = fc.integer({ min: 0, max: 10 });
const decimalArb = fc
  .tuple(unitsArb, scaleArb)
  .map(([units, scale]) => Decimal.fromUnits(units, scale));

describe('Decimal 属性', () => {
  it('fromUnits -> toString -> parse 往返保持数值', () => {
    fc.assert(
      fc.property(unitsArb, scaleArb, (units, scale) => {
        const value = Decimal.fromUnits(units, scale);
        expect(Decimal.parse(value.toString()).equals(value)).toBe(true);
        expect(Decimal.parse(value.toFixed(scale)).equals(value)).toBe(true);
      }),
      { numRuns: 200 },
    );
  });

  it('加法交换律与减法逆运算', () => {
    fc.assert(
      fc.property(decimalArb, decimalArb, (left, right) => {
        expect(left.add(right).equals(right.add(left))).toBe(true);
        expect(left.subtract(right).add(right).equals(left)).toBe(true);
      }),
      { numRuns: 200 },
    );
  });

  it('乘法交换律与乘零', () => {
    fc.assert(
      fc.property(decimalArb, decimalArb, (left, right) => {
        expect(left.multiply(right).equals(right.multiply(left))).toBe(true);
        expect(left.multiply(Decimal.ZERO).isZero()).toBe(true);
      }),
      { numRuns: 200 },
    );
  });

  it('比较与相等一致', () => {
    fc.assert(
      fc.property(decimalArb, decimalArb, (left, right) => {
        const comparison = left.compare(right);
        expect(left.equals(right)).toBe(comparison === 0);
        expect(comparison === -right.compare(left)).toBe(true);
      }),
      { numRuns: 200 },
    );
  });

  it('除法结果乘以除数后误差不超过 |除数| * 10^-18', () => {
    const denominatorArb = decimalArb.filter((value) => !value.isZero());
    fc.assert(
      fc.property(decimalArb, denominatorArb, (numerator, denominator) => {
        const quotient = numerator.divide(denominator, {
          scale: 18,
          rounding: 'half-even',
        });
        const reconstructed = quotient.multiply(denominator);
        const error = reconstructed.subtract(numerator).abs();
        const bound = denominator.abs().multiply(Decimal.parse('0.000000000000000001'));
        expect(error.compare(bound) <= 0).toBe(true);
      }),
      { numRuns: 200 },
    );
  });

  it('步长对齐结果一定是步长整数倍且误差不超过半个步长', () => {
    const positiveStepArb = fc
      .tuple(fc.bigInt({ min: 1n, max: 10n ** 8n }), fc.integer({ min: 0, max: 6 }))
      .map(([units, scale]) => Decimal.fromUnits(units, scale));
    fc.assert(
      fc.property(decimalArb, positiveStepArb, (value, step) => {
        const rounded = roundDecimalToStep(value, step, 'half-even');
        expect(isDecimalMultipleOf(rounded, step)).toBe(true);
        const error = rounded.subtract(value).abs();
        expect(error.compare(step) <= 0).toBe(true);
      }),
      { numRuns: 200 },
    );
  });

  it('toFixed 指定 scale 后再次解析得到同一舍入值', () => {
    fc.assert(
      fc.property(decimalArb, fc.integer({ min: 0, max: 10 }), (value, scale) => {
        const text = value.toFixed(scale, 'half-even');
        const parsed = Decimal.parse(text);
        expect(parsed.equals(value.rescale(scale, 'half-even'))).toBe(true);
        expect(text.split('.')[1]?.length ?? 0).toBe(scale === 0 ? 0 : scale);
      }),
      { numRuns: 200 },
    );
  });
});
