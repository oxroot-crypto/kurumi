/**
 * @file 衍生品指标单元测试：基差、资金费与期货名义价值。
 * @description 覆盖 FC-DER-*。
 * @module finance/derivatives.test
 */

import { describe, expect, it } from 'vitest';

import { Decimal } from '@kurumi/math';
import { FinanceDefinitionError, FinanceValueError } from './errors.js';
import {
  fundingPayment,
  futureNotional,
  futuresBasis,
  futuresBasisRate,
  futuresIsBackwardation,
  futuresIsContango,
} from './derivatives.js';
import { USD, btcPerp, btcUsdt, esFuture } from './fixtures.test-utils.js';

describe('期货基差', () => {
  it('基差、基差率、升贴水', () => {
    expect(futuresBasis(Decimal.parse('5100'), Decimal.parse('5000')).toString()).toBe('100');
    expect(futuresBasisRate(Decimal.parse('5100'), Decimal.parse('5000')).toString()).toBe('0.02');
    expect(futuresIsContango(Decimal.parse('5100'), Decimal.parse('5000'))).toBe(true);
    expect(futuresIsBackwardation(Decimal.parse('5100'), Decimal.parse('5000'))).toBe(false);
    expect(futuresIsBackwardation(Decimal.parse('4900'), Decimal.parse('5000'))).toBe(true);
    expect(futuresBasis(Decimal.parse('4900'), Decimal.parse('5000')).toString()).toBe('-100');
  });

  it('拒绝非正价格', () => {
    expect(() => futuresBasis(Decimal.ZERO, Decimal.ONE)).toThrow(FinanceDefinitionError);
    expect(() => futuresBasisRate(Decimal.ONE, Decimal.ZERO)).toThrow(FinanceDefinitionError);
  });
});

describe('资金费', () => {
  it('多头支付、空头收取、零仓位为零', () => {
    const long = fundingPayment({
      instrument: btcPerp(),
      netQuantity: Decimal.ONE,
      markPrice: Decimal.parse('100000'),
      fundingRate: Decimal.parse('0.0001'),
    });
    expect(long.amount.toString()).toBe('-10');
    expect(long.currency.code).toBe('USDT');

    const short = fundingPayment({
      instrument: btcPerp(),
      netQuantity: Decimal.parse('-1'),
      markPrice: Decimal.parse('100000'),
      fundingRate: Decimal.parse('0.0001'),
    });
    expect(short.amount.toString()).toBe('10');

    const zero = fundingPayment({
      instrument: btcPerp(),
      netQuantity: Decimal.ZERO,
      markPrice: Decimal.parse('100000'),
      fundingRate: Decimal.parse('0.0001'),
    });
    expect(zero.amount.isZero()).toBe(true);
  });

  it('负资金费率方向相反', () => {
    const long = fundingPayment({
      instrument: btcPerp(),
      netQuantity: Decimal.ONE,
      markPrice: Decimal.parse('100000'),
      fundingRate: Decimal.parse('-0.0001'),
    });
    expect(long.amount.toString()).toBe('10');
  });

  it('拒绝非永续标的、非正标记价与非法费率', () => {
    expect(() =>
      fundingPayment({
        instrument: btcUsdt() as never,
        netQuantity: Decimal.ONE,
        markPrice: Decimal.ONE,
        fundingRate: Decimal.ONE,
      }),
    ).toThrow(FinanceDefinitionError);
    expect(() =>
      fundingPayment({
        instrument: btcPerp(),
        netQuantity: Decimal.ONE,
        markPrice: Decimal.ZERO,
        fundingRate: Decimal.ONE,
      }),
    ).toThrow(FinanceValueError);
    expect(() =>
      fundingPayment({
        instrument: btcPerp(),
        netQuantity: Decimal.ONE,
        markPrice: Decimal.ONE,
        fundingRate: '0.1' as never,
      }),
    ).toThrow(FinanceDefinitionError);
  });
});

describe('期货名义价值', () => {
  it('按合约乘数计算', () => {
    expect(
      futureNotional(esFuture(), Decimal.parse('5000'), Decimal.parse('2')).amount.toString(),
    ).toBe('500000');
    expect(futureNotional(esFuture(), Decimal.parse('5000'), Decimal.ZERO).amount.isZero()).toBe(
      true,
    );
    expect(futureNotional(esFuture(), Decimal.parse('5000'), Decimal.ZERO).currency).toEqual(USD);
  });

  it('拒绝非正价格与负数量', () => {
    expect(() => futureNotional(esFuture(), Decimal.ZERO, Decimal.ONE)).toThrow(
      FinanceDefinitionError,
    );
    expect(() => futureNotional(esFuture(), Decimal.ONE, Decimal.parse('-1'))).toThrow(
      FinanceDefinitionError,
    );
  });
});
