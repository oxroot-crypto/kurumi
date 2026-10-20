/**
 * @file 期权单元测试：内在价值、货币性、Black-Scholes、Greeks 与隐含波动率。
 * @description 覆盖 FC-OPT-* 的已知数值、退化情形与错误边界。
 * @module finance/option.test
 */

import { describe, expect, it } from 'vitest';

import { Decimal } from '@kurumi/math';
import { FinanceDefinitionError, FinanceValueError, OptionModelError } from './errors.js';
import {
  ExerciseStyle,
  OptionType,
  SettlementType,
  createInstrument,
  type OptionInstrument,
} from './instrument.js';
import { createMoney } from './money.js';
import {
  blackScholes,
  blackScholesPrice,
  impliedVolatility,
  optionBreakeven,
  optionIntrinsicValue,
  optionIntrinsicValueMoney,
  optionIsInTheMoney,
  optionMoneyness,
  optionPayoffAtExpiry,
} from './option.js';
import { T0, USDT, btcCall } from './fixtures.test-utils.js';

function putOption(): OptionInstrument {
  return createInstrument({
    assetClass: 'option',
    venue: 'deribit',
    symbol: 'BTC-100000-P',
    quoteCurrency: USDT,
    tickSize: Decimal.parse('0.01'),
    lotSize: Decimal.parse('0.001'),
    optionType: OptionType.Put,
    strike: Decimal.parse('100000'),
    expiryMs: T0 + 1_000_000,
    exerciseStyle: ExerciseStyle.European,
    settlement: SettlementType.Cash,
    underlying: 'BTC/USDT',
  }) as OptionInstrument;
}

const baseInput = {
  spot: 100,
  strike: 100,
  timeToExpiryYears: 1,
  volatility: 0.2,
  riskFreeRate: 0.05,
  optionType: OptionType.Call,
} as const;

describe('内在价值与货币性', () => {
  it('call/put 内在价值、金额与实值判断', () => {
    const call = btcCall();
    const put = putOption();
    expect(optionIntrinsicValue(call, Decimal.parse('110000')).toString()).toBe('10000');
    expect(optionIntrinsicValue(call, Decimal.parse('90000')).isZero()).toBe(true);
    expect(optionIntrinsicValue(put, Decimal.parse('90000')).toString()).toBe('10000');
    expect(optionIntrinsicValue(put, Decimal.parse('110000')).isZero()).toBe(true);
    expect(optionIntrinsicValueMoney(call, Decimal.parse('110000')).amount.toString()).toBe(
      '10000',
    );
    expect(optionIntrinsicValueMoney(call, Decimal.parse('110000')).currency).toEqual(USDT);
    expect(optionIsInTheMoney(call, Decimal.parse('110000'))).toBe(true);
    expect(optionIsInTheMoney(call, Decimal.parse('90000'))).toBe(false);
    expect(optionIsInTheMoney(put, Decimal.parse('90000'))).toBe(true);
    expect(optionIsInTheMoney(put, Decimal.parse('110000'))).toBe(false);
    expect(optionMoneyness(call, Decimal.parse('110000')).toString()).toBe('1.1');
  });

  it('拒绝非法标的与价格', () => {
    const call = btcCall();
    expect(() => optionIntrinsicValue(call, Decimal.ZERO)).toThrow(FinanceValueError);
    expect(() => optionMoneyness(call, Decimal.ZERO)).toThrow();
    expect(() => optionIsInTheMoney(call, Decimal.ZERO)).toThrow();
    expect(() => optionIntrinsicValue({} as never, Decimal.ONE)).toThrow(FinanceDefinitionError);
  });

  it('盈亏平衡与到期损益', () => {
    const call = btcCall();
    const put = putOption();
    expect(optionBreakeven(call, Decimal.parse('500')).toString()).toBe('100500');
    expect(optionBreakeven(put, Decimal.parse('500')).toString()).toBe('99500');
    expect(() => optionBreakeven(call, Decimal.parse('-1'))).toThrow();
    expect(optionPayoffAtExpiry(call, Decimal.parse('110000')).amount.toString()).toBe('10000');
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
      optionType: OptionType.Put,
    });
    const parity = 100 * Math.exp(-0.02) - 100 * Math.exp(-0.05);
    expect(call.price - put.price).toBeCloseTo(parity, 4);
    expect(put.price).toBeGreaterThan(0);
  });

  it('T=0 与 sigma=0 退化', () => {
    const expiryItm = blackScholes({
      ...baseInput,
      timeToExpiryYears: 0,
      spot: 110,
    });
    expect(expiryItm.price).toBe(10);
    expect(expiryItm.delta).toBe(1);
    expect(expiryItm.gamma).toBe(0);
    expect(expiryItm.vega).toBe(0);

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
      optionType: OptionType.Put,
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
          optionType: OptionType.Call,
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
          optionType: OptionType.Call,
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
      optionType: OptionType.Call,
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
      optionType: OptionType.Put,
    } as const;
    const price = blackScholesPrice({ ...putInput, volatility: 0.3 });
    expect(impliedVolatility(putInput, price)).toBeCloseTo(0.3, 6);
  });

  it('期权金额工具与手续费币种联动', () => {
    expect(createMoney(Decimal.ONE, USDT).amount.toString()).toBe('1');
  });
});
