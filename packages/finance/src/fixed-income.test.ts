/**
 * @file 固定收益单元测试：付息日、应计利息、净价/全价、YTM、久期、凸性与 DV01。
 * @description 覆盖 FC-FI-* 与 I-FIN-016。
 * @module finance/fixed-income.test
 */

import { describe, expect, it } from 'vitest';

import { Decimal } from '@kurumi/math';
import { FinanceDefinitionError, FinanceValueError } from './errors.js';
import {
  CouponFrequency,
  createBondSpec,
  DayCountConvention,
  accruedInterest,
  bondCashFlows,
  bondCleanPrice,
  bondDirtyPrice,
  bondYield,
  convexity,
  couponPerPeriod,
  couponSchedule,
  currentYield,
  daysBetween,
  dv01,
  isBondSpec,
  macaulayDuration,
  modifiedDuration,
  yearFraction,
  type BondSpec,
} from './fixed-income.js';
import { createMoney, createCurrency } from './money.js';
import { USD } from './fixtures.test-utils.js';

const ISSUE = Date.UTC(2020, 0, 1);
const MATURITY = Date.UTC(2030, 0, 1);
const DAY_MS = 86_400_000;

function bond(overrides: Partial<BondSpec> = {}): BondSpec {
  return createBondSpec({
    id: 'bond-1',
    faceValue: Decimal.parse('1000'),
    couponRate: Decimal.parse('0.05'),
    couponFrequency: CouponFrequency.SemiAnnual,
    issueDateMs: ISSUE,
    maturityDateMs: MATURITY,
    dayCount: DayCountConvention.ActualActual,
    currency: USD,
    ...overrides,
  });
}

function d(value: string): Decimal {
  return Decimal.parse(value);
}

describe('createBondSpec / couponSchedule', () => {
  it('创建并校验债券规格', () => {
    const spec = bond();
    expect(isBondSpec(spec)).toBe(true);
    expect(Object.isFrozen(spec)).toBe(true);
    expect(() => bond({ faceValue: Decimal.ZERO })).toThrow(FinanceDefinitionError);
    expect(() => bond({ couponRate: d('-0.01') })).toThrow(FinanceDefinitionError);
    expect(() => bond({ couponFrequency: 3 as never })).toThrow(FinanceDefinitionError);
    expect(() => bond({ maturityDateMs: ISSUE })).toThrow(FinanceDefinitionError);
    expect(() => bond({ dayCount: 'actual/999' as never })).toThrow(FinanceDefinitionError);
    expect(() => bond({ currency: createCurrency('US', 2) })).toThrow(FinanceDefinitionError);
  });

  it('生成严格递增付息日并包含到期日', () => {
    const schedule = couponSchedule(bond());
    expect(schedule).toHaveLength(20);
    expect(schedule[0]).toBe(Date.UTC(2020, 6, 1));
    expect(schedule[schedule.length - 1]).toBe(MATURITY);
    for (let index = 1; index < schedule.length; index += 1) {
      expect(schedule[index]).toBeGreaterThan(schedule[index - 1] ?? 0);
    }
    expect(() => couponSchedule({ ...bond(), faceValue: Decimal.ZERO })).toThrow();
  });

  it('非对齐到期日追加最后一期', () => {
    const schedule = couponSchedule(bond({ maturityDateMs: Date.UTC(2021, 2, 15) }));
    expect(schedule[schedule.length - 1]).toBe(Date.UTC(2021, 2, 15));
  });
});

describe('日计数与票息', () => {
  it('daysBetween/yearFraction', () => {
    const april = Date.UTC(2020, 3, 1);
    expect(daysBetween(ISSUE, april)).toBe(91);
    expect(yearFraction(ISSUE, april, DayCountConvention.Actual360)).toBeCloseTo(91 / 360, 12);
    expect(yearFraction(ISSUE, april, DayCountConvention.Actual365)).toBeCloseTo(91 / 365, 12);
    expect(yearFraction(ISSUE, april, DayCountConvention.ActualActual)).toBeCloseTo(
      91 / 365.25,
      12,
    );
    expect(yearFraction(ISSUE, april, DayCountConvention.Thirty360)).toBeCloseTo(0.25, 12);
  });

  it('couponPerPeriod 与零息债', () => {
    expect(couponPerPeriod(bond()).amount.toString()).toBe('25');
    expect(couponPerPeriod(bond({ couponRate: Decimal.ZERO })).amount.isZero()).toBe(true);
  });
});

describe('应计利息与现金流', () => {
  it('发行日应计为零，期间按天数比例计算', () => {
    expect(accruedInterest(bond(), ISSUE).amount.isZero()).toBe(true);
    expect(accruedInterest(bond(), Date.UTC(2020, 3, 1)).amount.toString()).toBe('12.5');
    expect(accruedInterest(bond(), Date.UTC(2020, 5, 30)).amount.toNumber()).toBeCloseTo(
      24.86263736,
      6,
    );
  });

  it('现金流包含最后本金', () => {
    const flows = bondCashFlows(bond(), ISSUE);
    expect(flows).toHaveLength(20);
    expect(flows[0]?.coupon.amount.toString()).toBe('25');
    expect(flows[0]?.principal.amount.isZero()).toBe(true);
    expect(flows[flows.length - 1]?.principal.amount.toString()).toBe('1000');
    expect(flows[flows.length - 1]?.total.amount.toString()).toBe('1025');
    expect(Object.isFrozen(flows)).toBe(true);
  });

  it('结算日越界失败', () => {
    expect(() => accruedInterest(bond(), ISSUE - DAY_MS)).toThrow(FinanceValueError);
    expect(() => bondCashFlows(bond(), MATURITY)).toThrow(FinanceValueError);
  });
});

describe('债券定价与 YTM', () => {
  it('零收益率时净价等于全部现金流，扣除应计后为净价', () => {
    expect(bondDirtyPrice(bond(), ISSUE, Decimal.ZERO).amount.toString()).toBe('1500');
    expect(bondCleanPrice(bond(), ISSUE, Decimal.ZERO).amount.toString()).toBe('1500');
    const dirty = bondDirtyPrice(bond(), Date.UTC(2020, 3, 1), Decimal.ZERO);
    expect(dirty.amount.toString()).toBe('1500');
    expect(bondCleanPrice(bond(), Date.UTC(2020, 3, 1), Decimal.ZERO).amount.toString()).toBe(
      '1487.5',
    );
  });

  it('YTM 可反解净价', () => {
    const settlement = ISSUE;
    const target = bondCleanPrice(bond(), settlement, d('0.06'));
    const ytm = bondYield(bond(), settlement, target);
    expect(ytm.toNumber()).toBeCloseTo(0.06, 6);
    expect(currentYield(bond(), createMoney(d('1000'), USD)).toString()).toBe('0.05');
  });

  it('久期、修正久期、凸性与 DV01 为正', () => {
    const settlement = ISSUE;
    const yieldRate = d('0.05');
    const macaulay = macaulayDuration(bond(), settlement, yieldRate);
    const modified = modifiedDuration(bond(), settlement, yieldRate);
    expect(macaulay.toNumber()).toBeGreaterThan(0);
    expect(modified.toNumber()).toBeGreaterThan(0);
    expect(modified.toNumber()).toBeLessThan(macaulay.toNumber());
    expect(convexity(bond(), settlement, yieldRate).toNumber()).toBeGreaterThan(0);
    expect(dv01(bond(), settlement, yieldRate).amount.toNumber()).toBeGreaterThan(0);
  });

  it('高精度面值可以定价，不会因自然 scale 超限失败', () => {
    const highPrecision = bond({
      id: 'high-precision',
      faceValue: d('1000.123456789012345678901234567890'),
    });
    expect(() => bondDirtyPrice(highPrecision, ISSUE, d('0.05'))).not.toThrow();
  });

  it('零息债定价与 YTM', () => {
    const zero = bond({ couponRate: Decimal.ZERO, id: 'zero' });
    const price = bondCleanPrice(zero, ISSUE, d('0.05'));
    expect(price.amount.toNumber()).toBeCloseTo(
      1000 / Math.pow(1.05, yearFraction(ISSUE, MATURITY, DayCountConvention.ActualActual)),
      6,
    );
    expect(bondYield(zero, ISSUE, price).toNumber()).toBeCloseTo(0.05, 6);
  });

  it('拒绝非法收益率、价格与结算日', () => {
    expect(() => bondDirtyPrice(bond(), ISSUE, d('-1'))).toThrow(FinanceValueError);
    expect(() => bondYield(bond(), ISSUE, createMoney(Decimal.ZERO, USD))).toThrow(
      FinanceValueError,
    );
    expect(() => bondYield(bond(), ISSUE, createMoney(d('0.000000001'), USD))).toThrow(
      FinanceValueError,
    );
    const eur = createCurrency('EUR', 2);
    expect(() => bondYield(bond(), ISSUE, createMoney(d('1000'), eur))).toThrow();
    expect(() => currentYield(bond(), createMoney(d('1000'), eur))).toThrow();
    expect(() => currentYield(bond(), createMoney(Decimal.ZERO, USD))).toThrow(FinanceValueError);
    expect(() => bondCleanPrice(bond(), MATURITY, d('0.05'))).toThrow(FinanceValueError);
  });
});
