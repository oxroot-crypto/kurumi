/**
 * @file Decimal 单元测试：解析、规范化、四则、舍入、步长与边界。
 * @description 覆盖 FC-DEC-* 全部约束与错误语义。
 * @module math/decimal.test
 */

import { describe, expect, it } from 'vitest';

import {
  Decimal,
  DecimalRounding,
  MAX_DECIMAL_DIGITS,
  MAX_DECIMAL_INPUT_LENGTH,
  MAX_DECIMAL_SCALE,
  isDecimal,
  isDecimalMultipleOf,
  maxDecimal,
  minDecimal,
  roundDecimalToStep,
  sumDecimals,
} from './decimal.js';
import {
  DecimalDivisionByZeroError,
  DecimalOverflowError,
  DecimalParseError,
  DecimalRoundingError,
  MathDefinitionError,
} from './errors.js';

function d(value: string): Decimal {
  return Decimal.parse(value);
}

describe('Decimal.parse', () => {
  it('解析普通、符号、前导小数、指数与尾随零', () => {
    expect(d('123.45').toString()).toBe('123.45');
    expect(d('-0.001').toString()).toBe('-0.001');
    expect(d('.5').toString()).toBe('0.5');
    expect(d('+1').toString()).toBe('1');
    expect(d('1e2').toString()).toBe('100');
    expect(d('1.5e-2').toString()).toBe('0.015');
    expect(d('1.2300e2').toString()).toBe('123');
    expect(d('1.2300').toString()).toBe('1.23');
    expect(d('000123.4500').toString()).toBe('123.45');
    expect(d('-0').toString()).toBe('0');
    expect(d('0.0000').toString()).toBe('0');
    expect(d('0e-100').toString()).toBe('0');
  });

  it('拒绝非法语法', () => {
    for (const input of [
      '',
      ' ',
      'NaN',
      'Infinity',
      '-Infinity',
      '0x10',
      '1_000',
      '1.',
      '.',
      '1e',
      '1e+',
      '1e1.5',
      '1,000',
      ' 1',
      '1 ',
      '+',
      '-',
      '1ee2',
    ]) {
      expect(() => Decimal.parse(input), input).toThrow(DecimalParseError);
    }
  });

  it('拒绝 scale 超限与超长输入/指数', () => {
    expect(() => d('1e-31')).toThrow(DecimalOverflowError);
    expect(() => d('1e1001')).toThrow(DecimalParseError);
    expect(() => d('1'.repeat(MAX_DECIMAL_INPUT_LENGTH + 1))).toThrow(DecimalParseError);
  });

  it('接受 scale 上界与位数上界', () => {
    expect(d('1e-30').toString()).toBe(`0.${'0'.repeat(29)}1`);
    const digits = '1' + '0'.repeat(MAX_DECIMAL_DIGITS - 1);
    expect(d(digits).toString()).toBe(digits);
  });

  it('拒绝超过位数上限的数值', () => {
    expect(() => d('1' + '0'.repeat(MAX_DECIMAL_DIGITS))).toThrow(DecimalOverflowError);
  });
});

describe('Decimal.fromUnits', () => {
  it('规范化尾随零与零值', () => {
    expect(Decimal.fromUnits(1500n, 2).toString()).toBe('15');
    expect(Decimal.fromUnits(0n, 10).toString()).toBe('0');
    expect(Decimal.fromUnits(5n, 2).toString()).toBe('0.05');
    expect(Decimal.fromUnits(-5n, 2).toString()).toBe('-0.05');
  });

  it('拒绝非法 scale', () => {
    expect(() => Decimal.fromUnits(1n, -1)).toThrow(DecimalOverflowError);
    expect(() => Decimal.fromUnits(1n, MAX_DECIMAL_SCALE + 1)).toThrow(DecimalOverflowError);
    expect(() => Decimal.fromUnits(1n, 1.5)).toThrow(DecimalOverflowError);
  });

  it('fromInteger 只接受安全整数', () => {
    expect(Decimal.fromInteger(42).toString()).toBe('42');
    expect(Decimal.fromInteger(-42n).toString()).toBe('-42');
    expect(() => Decimal.fromInteger(Number.MAX_SAFE_INTEGER + 1)).toThrow(DecimalOverflowError);
  });
});

describe('Decimal 算术', () => {
  it('加减对齐 scale 且精确', () => {
    expect(d('1.2').add(d('3.45')).toString()).toBe('4.65');
    expect(d('1.2').subtract(d('3.45')).toString()).toBe('-2.25');
    expect(d('1').add(d('-1')).toString()).toBe('0');
  });

  it('乘法在自然 scale 内精确', () => {
    expect(d('1.5').multiply(d('2.4')).toString()).toBe('3.6');
    expect(d('-1.5').multiply(d('0')).toString()).toBe('0');
  });

  it('乘法显式 scale 时按模式舍入', () => {
    expect(d('1.005').multiply(d('1'), { scale: 2, rounding: 'half-even' }).toString()).toBe('1');
    expect(d('1.005').multiply(d('1'), { scale: 2, rounding: 'half-up' }).toString()).toBe('1.01');
    expect(() => d('1.005').multiply(d('1'), { scale: 2 })).toThrow(DecimalRoundingError);
  });

  it('乘法自然 scale 超过上限时要求显式 scale', () => {
    expect(() => d('1e-20').multiply(d('1e-20'))).toThrow(DecimalOverflowError);
    expect(d('1e-20').multiply(d('1e-20'), { scale: 30, rounding: 'down' }).toString()).toBe('0');
  });

  it('除法默认 scale 18、half-even', () => {
    expect(d('1').divide(d('3')).toString()).toBe('0.333333333333333333');
    expect(d('2').divide(d('3')).toString()).toBe('0.666666666666666667');
    expect(d('-1').divide(d('3')).toString()).toBe('-0.333333333333333333');
  });

  it('除法支持显式 scale 与全部舍入模式', () => {
    expect(d('1').divide(d('8'), { scale: 2 }).toString()).toBe('0.12');
    expect(d('1').divide(d('8'), { scale: 2, rounding: DecimalRounding.Down }).toString()).toBe(
      '0.12',
    );
    expect(d('1').divide(d('8'), { scale: 2, rounding: DecimalRounding.Up }).toString()).toBe(
      '0.13',
    );
    expect(d('1').divide(d('8'), { scale: 2, rounding: DecimalRounding.Ceil }).toString()).toBe(
      '0.13',
    );
    expect(d('-1').divide(d('8'), { scale: 2, rounding: DecimalRounding.Floor }).toString()).toBe(
      '-0.13',
    );
    expect(d('1').divide(d('8'), { scale: 2, rounding: DecimalRounding.HalfUp }).toString()).toBe(
      '0.13',
    );
    expect(d('1').divide(d('4'), { scale: 2, rounding: DecimalRounding.Exact }).toString()).toBe(
      '0.25',
    );
    expect(() => d('1').divide(d('3'), { scale: 2, rounding: DecimalRounding.Exact })).toThrow(
      DecimalRoundingError,
    );
    expect(() => d('1').divide(d('8'), { scale: 2, rounding: DecimalRounding.Exact })).toThrow(
      DecimalRoundingError,
    );
  });

  it('除零与非法 scale 失败', () => {
    expect(() => d('1').divide(d('0'))).toThrow(DecimalDivisionByZeroError);
    expect(() => d('1').divide(d('2'), { scale: -1 })).toThrow(DecimalOverflowError);
    expect(() => d('1').divide(d('2'), { scale: 31 })).toThrow(DecimalOverflowError);
  });

  it('非法舍入模式是定义错误而不是数值溢出', () => {
    expect(() =>
      d('1').divide(d('2'), {
        // @ts-expect-error - 测试运行期非法枚举值
        rounding: 'bogus',
      }),
    ).toThrow(MathDefinitionError);
  });

  it('rescale / round / toFixed', () => {
    expect(d('1.2345').rescale(2, 'half-even').toString()).toBe('1.23');
    expect(d('1.2355').rescale(2, 'half-even').toString()).toBe('1.24');
    expect(d('1.2345').round(2).toString()).toBe('1.23');
    expect(d('-1.25').toFixed(1, 'half-even')).toBe('-1.2');
    expect(d('-1.25').toFixed(1, 'half-up')).toBe('-1.3');
    expect(d('-0.005').toFixed(2, 'half-even')).toBe('0.00');
    expect(d('1').toFixed(3)).toBe('1.000');
    expect(() => d('1.234').rescale(2)).toThrow(DecimalRoundingError);
  });

  it('比较、符号、绝对值与相等', () => {
    expect(d('1.2').compare(d('1.20'))).toBe(0);
    expect(d('-1').compare(d('0'))).toBe(-1);
    expect(d('2').compare(d('1.999'))).toBe(1);
    expect(d('-2').sign()).toBe(-1);
    expect(d('0').sign()).toBe(0);
    expect(d('2').sign()).toBe(1);
    expect(d('-2').abs().toString()).toBe('2');
    expect(d('2').negate().toString()).toBe('-2');
    expect(d('0').negate().toString()).toBe('0');
    expect(d('0').isZero()).toBe(true);
    expect(d('1').isInteger()).toBe(true);
    expect(d('1.1').isInteger()).toBe(false);
    expect(d('1').equals(d('1.0'))).toBe(true);
  });

  it('toNumber 有损但拒绝溢出', () => {
    expect(d('1.5').toNumber()).toBe(1.5);
    expect(() => d('1e1000').toNumber()).toThrow(DecimalOverflowError);
  });

  it('toJSON 输出字符串', () => {
    expect(JSON.stringify({ value: d('1.20') })).toBe('{"value":"1.2"}');
  });
});

describe('Decimal 集合与步长', () => {
  it('sum/min/max', () => {
    expect(sumDecimals([d('1.2'), d('2.3'), d('3.5')]).toString()).toBe('7');
    expect(Decimal.sum([]).toString()).toBe('0');
    expect(minDecimal([d('2'), d('-1'), d('3')]).toString()).toBe('-1');
    expect(maxDecimal([d('2'), d('-1'), d('3')]).toString()).toBe('3');
    expect(() => minDecimal([])).toThrow(MathDefinitionError);
    expect(() => maxDecimal([])).toThrow(MathDefinitionError);
  });

  it('isDecimalMultipleOf 支持正负与不同 scale', () => {
    expect(isDecimalMultipleOf(d('100.00'), d('0.01'))).toBe(true);
    expect(isDecimalMultipleOf(d('-100.00'), d('0.01'))).toBe(true);
    expect(isDecimalMultipleOf(d('100.005'), d('0.01'))).toBe(false);
    expect(isDecimalMultipleOf(d('0'), d('0.01'))).toBe(true);
    expect(() => isDecimalMultipleOf(d('1'), d('0'))).toThrow(MathDefinitionError);
  });

  it('roundDecimalToStep 各舍入模式', () => {
    expect(roundDecimalToStep(d('100.126'), d('0.05')).toString()).toBe('100.15');
    expect(roundDecimalToStep(d('100.124'), d('0.05')).toString()).toBe('100.1');
    expect(roundDecimalToStep(d('-100.126'), d('0.05'), 'floor').toString()).toBe('-100.15');
    expect(() => roundDecimalToStep(d('100.126'), d('0.05'), 'exact')).toThrow(
      DecimalRoundingError,
    );
  });

  it('isDecimal 识别实例', () => {
    expect(isDecimal(d('1'))).toBe(true);
    expect(isDecimal('1')).toBe(false);
    expect(isDecimal(null)).toBe(false);
  });
});
