/**
 * @file 期权定价数学单元测试。
 * @description 覆盖内在价值、货币性、Black-Scholes、Greeks、隐含波动率与错误边界。
 * @module math/option-math.test
 */

import { describe, expect, it } from 'vitest';

import { Decimal } from './decimal.js';
import { MathValueError, OptionModelError } from './errors.js';
import {
  OptionKind,
  blackScholes,
  blackScholesPrice,
  impliedVolatility,
  optionBreakeven,
  optionIntrinsicValue,
  optionIsInTheMoney,
  optionMoneyness,
} from './option-math.js';

const baseInput = {
  spot: 100,
  strike: 100,
  timeToExpiryYears: 1,
  volatility: 0.2,
  riskFreeRate: 0.05,
  optionType: OptionKind.Call,
} as const;

describe('内在价值与货币性', () => {
  it('call/put 内在价值、货币性与实值判断', () => {
    expect(
      optionIntrinsicValue(OptionKind.Call, Decimal.parse('110'), Decimal.parse('100')).toString(),
    ).toBe('10');
    expect(
      optionIntrinsicValue(OptionKind.Call, Decimal.parse('90'), Decimal.parse('100')).isZero(),
    ).toBe(true);
    expect(
      optionIntrinsicValue(OptionKind.Put, Decimal.parse('90'), Decimal.parse('100')).toString(),
    ).toBe('10');
    expect(optionMoneyness(Decimal.parse('110'), Decimal.parse('100')).toString()).toBe('1.1');
    expect(optionIsInTheMoney(OptionKind.Call, Decimal.parse('110'), Decimal.parse('100'))).toBe(
      true,
    );
    expect(optionIsInTheMoney(OptionKind.Put, Decimal.parse('90'), Decimal.parse('100'))).toBe(
      true,
    );
  });

  it('盈亏平衡与非法参数', () => {
    expect(
      optionBreakeven(OptionKind.Call, Decimal.parse('100'), Decimal.parse('5')).toString(),
    ).toBe('105');
    expect(
      optionBreakeven(OptionKind.Put, Decimal.parse('100'), Decimal.parse('5')).toString(),
    ).toBe('95');
    expect(() => optionIntrinsicValue(OptionKind.Call, Decimal.ZERO, Decimal.ONE)).toThrow(
      MathValueError,
    );
    expect(() => optionMoneyness(Decimal.ONE, Decimal.ZERO)).toThrow(MathValueError);
    expect(() => optionBreakeven(OptionKind.Call, Decimal.ONE, Decimal.parse('-1'))).toThrow(
      MathValueError,
    );
    expect(() => optionIntrinsicValue('straddle' as never, Decimal.ONE, Decimal.ONE)).toThrow(
      MathValueError,
    );
  });
});

describe('Black-Scholes', () => {
  it('已知平价看涨期权数值', () => {
    const result = blackScholes(baseInput);
    expect(result.price).toBeCloseTo(10.4506, 3);
    expect(result.delta).toBeCloseTo(0.6368, 3);
    expect(result.gamma).toBeCloseTo(0.018762, 4);
    expect(result.vega).toBeCloseTo(37.524, 2);
    expect(result.theta).toBeCloseTo(-6.414, 2);
    expect(result.rho).toBeCloseTo(53.232, 2);
    expect(blackScholesPrice(baseInput)).toBeCloseTo(10.4506, 3);
  });

  it('看跌期权与 put-call parity', () => {
    const call = blackScholes({ ...baseInput, dividendYield: 0.02 });
    const put = blackScholes({
      ...baseInput,
      dividendYield: 0.02,
      optionType: OptionKind.Put,
    });
    const parity = 100 * Math.exp(-0.02) - 100 * Math.exp(-0.05);
    expect(call.price - put.price).toBeCloseTo(parity, 4);
  });

  it('T=0 与 sigma=0 退化', () => {
    const expiryItm = blackScholes({ ...baseInput, timeToExpiryYears: 0, spot: 110 });
    expect(expiryItm.price).toBe(10);
    expect(expiryItm.delta).toBe(1);
    expect(expiryItm.gamma).toBe(0);

    const expiryAtm = blackScholes({ ...baseInput, timeToExpiryYears: 0 });
    expect(expiryAtm.price).toBe(0);
    expect(expiryAtm.delta).toBe(0.5);

    const zeroVol = blackScholes({ ...baseInput, volatility: 0 });
    expect(zeroVol.price).toBeCloseTo(100 - 100 * Math.exp(-0.05), 10);
    expect(zeroVol.gamma).toBe(0);
    expect(zeroVol.vega).toBe(0);
    expect(zeroVol.theta).toBe(0);

    const zeroVolPut = blackScholes({
      ...baseInput,
      volatility: 0,
      optionType: OptionKind.Put,
    });
    expect(zeroVolPut.price).toBe(0);
    expect(zeroVolPut.delta).toBe(0);
  });

  it('拒绝非法参数', () => {
    expect(() => blackScholes({ ...baseInput, spot: 0 })).toThrow(OptionModelError);
    expect(() => blackScholes({ ...baseInput, strike: -1 })).toThrow(OptionModelError);
    expect(() => blackScholes({ ...baseInput, timeToExpiryYears: -1 })).toThrow(OptionModelError);
    expect(() => blackScholes({ ...baseInput, volatility: -0.1 })).toThrow(OptionModelError);
    expect(() => blackScholes({ ...baseInput, riskFreeRate: Number.NaN })).toThrow(
      OptionModelError,
    );
    expect(() => blackScholes({ ...baseInput, optionType: 'straddle' as never })).toThrow(
      OptionModelError,
    );
  });
});

describe('隐含波动率', () => {
  it('从 Black-Scholes 价格反解波动率', () => {
    const price = blackScholesPrice({ ...baseInput, volatility: 0.25 });
    const iv = impliedVolatility(
      {
        spot: baseInput.spot,
        strike: baseInput.strike,
        timeToExpiryYears: baseInput.timeToExpiryYears,
        riskFreeRate: baseInput.riskFreeRate,
        optionType: baseInput.optionType,
      },
      price,
    );
    expect(iv).toBeCloseTo(0.25, 6);
  });

  it('零价格/到期/下界返回零', () => {
    expect(
      impliedVolatility(
        {
          spot: 80,
          strike: 100,
          timeToExpiryYears: 1,
          riskFreeRate: 0.05,
          optionType: OptionKind.Call,
        },
        0,
      ),
    ).toBe(0);
    expect(
      impliedVolatility(
        {
          spot: 110,
          strike: 100,
          timeToExpiryYears: 0,
          riskFreeRate: 0.05,
          optionType: OptionKind.Call,
        },
        10,
      ),
    ).toBe(0);
  });

  it('价格越界、maxVolatility 过低、参数非法与未收敛失败', () => {
    const input = {
      spot: 100,
      strike: 100,
      timeToExpiryYears: 1,
      riskFreeRate: 0.05,
      optionType: OptionKind.Call,
    } as const;
    expect(() => impliedVolatility(input, 1000)).toThrow(OptionModelError);
    expect(() => impliedVolatility(input, -1)).toThrow(OptionModelError);
    expect(() => impliedVolatility(input, 10, { maxVolatility: 0.01 })).toThrow(OptionModelError);
    expect(() => impliedVolatility(input, 10, { tolerance: 0 })).toThrow(OptionModelError);
    expect(() => impliedVolatility(input, 10, { maxIterations: 0 })).toThrow(OptionModelError);
    expect(() => impliedVolatility(input, 10, { maxIterations: 1, tolerance: 1e-12 })).toThrow(
      OptionModelError,
    );
    expect(() => impliedVolatility({ ...input, optionType: 'straddle' as never }, 10)).toThrow(
      OptionModelError,
    );
  });

  it('put 隐含波动率可反解', () => {
    const putInput = {
      spot: 100,
      strike: 100,
      timeToExpiryYears: 1,
      riskFreeRate: 0.05,
      optionType: OptionKind.Put,
    } as const;
    const price = blackScholesPrice({ ...putInput, volatility: 0.3 });
    expect(impliedVolatility(putInput, price)).toBeCloseTo(0.3, 6);
  });
});
