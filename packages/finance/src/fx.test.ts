/**
 * @file FX 单元测试：直接/反向/多跳、冲突检测、换算舍入与错误。
 * @description 覆盖 FC-FX-* 与确定性 BFS。
 * @module finance/fx.test
 */

import { describe, expect, it } from 'vitest';

import { Decimal } from '@kurumi/math';
import { CurrencyMismatchError, MissingFxRateError, FxRateError } from './errors.js';
import {
  convertMoneyWithRate,
  createFxConverter,
  createFxRate,
  invertFxRate,
  isFxRate,
} from './fx.js';
import { createCurrency, parseMoney } from './money.js';
import { BTC, EUR, JPY, T0, USD } from './fixtures.test-utils.js';

const eurUsd = createFxRate({
  base: EUR,
  quote: USD,
  rate: Decimal.parse('1.1'),
  timestampMs: T0,
});
const usdJpy = createFxRate({
  base: USD,
  quote: JPY,
  rate: Decimal.parse('150'),
  timestampMs: T0,
});

describe('createFxRate / invertFxRate', () => {
  it('创建并冻结汇率', () => {
    expect(eurUsd.rate.toString()).toBe('1.1');
    expect(Object.isFrozen(eurUsd)).toBe(true);
    expect(isFxRate(eurUsd)).toBe(true);
    expect(isFxRate({ ...eurUsd, rate: Decimal.ZERO })).toBe(false);
    expect(isFxRate({ ...eurUsd, base: USD })).toBe(false);
  });

  it('反转汇率取倒数', () => {
    const inverted = invertFxRate(eurUsd);
    expect(inverted.base).toEqual(USD);
    expect(inverted.quote).toEqual(EUR);
    expect(inverted.rate.toString()).toBe('0.909090909090909091');
    expect(() => invertFxRate({ ...eurUsd, rate: Decimal.ZERO })).toThrow(FxRateError);
  });

  it('拒绝同币种、非正汇率、非法时间', () => {
    expect(() =>
      createFxRate({ base: USD, quote: USD, rate: Decimal.ONE, timestampMs: T0 }),
    ).toThrow(FxRateError);
    expect(() =>
      createFxRate({ base: EUR, quote: USD, rate: Decimal.ZERO, timestampMs: T0 }),
    ).toThrow(FxRateError);
    expect(() =>
      createFxRate({ base: EUR, quote: USD, rate: Decimal.ONE, timestampMs: -1 }),
    ).toThrow();
  });
});

describe('createFxConverter', () => {
  it('直接、反向、多跳与确定性', () => {
    const fx = createFxConverter([eurUsd, usdJpy]);
    expect(fx.rate(EUR, USD).toString()).toBe('1.1');
    expect(fx.rate(USD, EUR).toString()).toBe('0.909090909090909091');
    expect(fx.rate(EUR, JPY).toString()).toBe('165');
    expect(fx.rate(JPY, EUR).toString()).toBe('0.006060606060606061');
    expect(fx.rate(EUR, EUR).toString()).toBe('1');

    const reordered = createFxConverter([usdJpy, eurUsd]);
    expect(reordered.rate(EUR, JPY).equals(fx.rate(EUR, JPY))).toBe(true);
    expect(fx.rates).toHaveLength(2);
    expect(Object.isFrozen(fx)).toBe(true);
  });

  it('换算金额并按目标 decimals 舍入', () => {
    const fx = createFxConverter([eurUsd, usdJpy]);
    expect(fx.convert(parseMoney('100', EUR), USD).amount.toString()).toBe('110');
    expect(fx.convert(parseMoney('100', USD), JPY).amount.toString()).toBe('15000');
    expect(
      fx.convert(parseMoney('1', EUR), USD, { scale: 4, rounding: 'down' }).amount.toString(),
    ).toBe('1.1');
    expect(fx.convert(parseMoney('10', USD), USD).amount.toString()).toBe('10');
    expect(() => fx.convert(parseMoney('1', EUR), USD, { scale: 31 })).toThrow(FxRateError);
  });

  it('同 code 不同 decimals 视为币种不一致', () => {
    const fx = createFxConverter([eurUsd]);
    const otherUsd = createCurrency('USD', 4);
    expect(() => fx.rate(USD, otherUsd)).toThrow(CurrencyMismatchError);
    expect(() => fx.convert(parseMoney('1', USD), otherUsd)).toThrow(CurrencyMismatchError);

    // 跨币种查询也不能因为 code 相同而命中缓存。
    expect(fx.rate(EUR, USD).toString()).toBe('1.1');
    const otherEur = createCurrency('EUR', 4);
    expect(() => fx.rate(otherEur, USD)).toThrow(CurrencyMismatchError);

    expect(() =>
      createFxConverter([
        eurUsd,
        createFxRate({
          base: otherEur,
          quote: JPY,
          rate: Decimal.parse('160'),
          timestampMs: T0,
        }),
      ]),
    ).toThrow(CurrencyMismatchError);
  });

  it('无路径失败', () => {
    const fx = createFxConverter([eurUsd]);
    expect(() => fx.rate(EUR, BTC)).toThrow(MissingFxRateError);
    expect(() => fx.convert(parseMoney('1', EUR), BTC)).toThrow(MissingFxRateError);
  });

  it('拒绝重复、反向冲突与非法汇率', () => {
    expect(() => createFxConverter([eurUsd, eurUsd])).toThrow(FxRateError);
    expect(() =>
      createFxConverter([
        eurUsd,
        createFxRate({
          base: USD,
          quote: EUR,
          rate: Decimal.parse('0.9'),
          timestampMs: T0,
        }),
      ]),
    ).toThrow(FxRateError);
    expect(() => createFxConverter([{ ...eurUsd, rate: Decimal.ZERO }])).toThrow(FxRateError);
  });

  it('空转换器对任意跨币种失败', () => {
    const fx = createFxConverter([]);
    expect(() => fx.rate(EUR, USD)).toThrow(MissingFxRateError);
    expect(fx.rate(EUR, EUR).toString()).toBe('1');
  });
});

describe('convertMoneyWithRate', () => {
  it('按单条汇率换算', () => {
    expect(convertMoneyWithRate(parseMoney('100', EUR), eurUsd).amount.toString()).toBe('110');
    expect(
      convertMoneyWithRate(parseMoney('100', EUR), eurUsd, {
        scale: 0,
        rounding: 'down',
      }).amount.toString(),
    ).toBe('110');
  });

  it('拒绝金额币种与 rate.base 不一致', () => {
    expect(() => convertMoneyWithRate(parseMoney('100', USD), eurUsd)).toThrow(
      CurrencyMismatchError,
    );
    expect(() =>
      convertMoneyWithRate(parseMoney('100', EUR), { ...eurUsd, rate: Decimal.ZERO }),
    ).toThrow(FxRateError);
  });
});
