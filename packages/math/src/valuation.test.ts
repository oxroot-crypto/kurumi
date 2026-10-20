/**
 * @file 估值模型单元测试：CAPM、WACC、DCF、Gordon、NPV/IRR、每股价值与安全边际。
 * @description 覆盖 FC-VAL-* 的已知数值与错误边界。
 * @module math/valuation.test
 */

import { describe, expect, it } from 'vitest';

import { MathValueError } from './errors.js';
import {
  capitalAssetPricingModel,
  discountedCashFlow,
  discountFactor,
  equityValueFromEnterpriseValue,
  gordonGrowthModel,
  internalRateOfReturn,
  marginOfSafety,
  netPresentValue,
  perShareValue,
  weightedAverageCostOfCapital,
} from './valuation.js';

describe('CAPM / WACC', () => {
  it('计算权益成本与加权资本成本', () => {
    expect(
      capitalAssetPricingModel({ riskFreeRate: 0.03, beta: 1.2, marketReturn: 0.08 }),
    ).toBeCloseTo(0.09, 10);
    expect(
      weightedAverageCostOfCapital({
        equityValue: 600,
        debtValue: 400,
        costOfEquity: 0.1,
        costOfDebt: 0.05,
        taxRate: 0.25,
      }),
    ).toBeCloseTo(0.075, 10);
  });

  it('拒绝非法参数', () => {
    expect(() =>
      capitalAssetPricingModel({ riskFreeRate: Number.NaN, beta: 1, marketReturn: 0.1 }),
    ).toThrow(MathValueError);
    expect(() =>
      weightedAverageCostOfCapital({
        equityValue: 0,
        debtValue: 0,
        costOfEquity: 0.1,
        costOfDebt: 0.05,
        taxRate: 0.25,
      }),
    ).toThrow(MathValueError);
    expect(() =>
      weightedAverageCostOfCapital({
        equityValue: 100,
        debtValue: 100,
        costOfEquity: 0.1,
        costOfDebt: 0.05,
        taxRate: 1.1,
      }),
    ).toThrow(MathValueError);
  });
});

describe('DCF / Gordon', () => {
  it('计算预测期现值、终值现值与企业价值', () => {
    const result = discountedCashFlow({
      cashFlows: [100, 110, 121],
      discountRate: 0.1,
      terminalGrowthRate: 0.02,
    });
    const expectedPv = 100 / 1.1 + 110 / 1.21 + 121 / 1.331;
    const terminal = (121 * 1.02) / (0.1 - 0.02);
    expect(result.presentValue).toBeCloseTo(expectedPv, 6);
    expect(result.terminalValuePresentValue).toBeCloseTo(terminal / 1.331, 6);
    expect(result.enterpriseValue).toBeCloseTo(expectedPv + terminal / 1.331, 6);

    const explicit = discountedCashFlow({
      cashFlows: [100],
      discountRate: 0.1,
      terminalValue: 1000,
    });
    expect(explicit.terminalValuePresentValue).toBeCloseTo(1000 / 1.1, 10);
  });

  it('Gordon 增长模型', () => {
    expect(
      gordonGrowthModel({ dividendPerShare: 2, requiredReturn: 0.08, growthRate: 0.03 }),
    ).toBeCloseTo(40, 10);
    expect(() =>
      gordonGrowthModel({ dividendPerShare: 2, requiredReturn: 0.03, growthRate: 0.03 }),
    ).toThrow(MathValueError);
  });

  it('拒绝非法 DCF 参数', () => {
    expect(() => discountedCashFlow({ cashFlows: [], discountRate: 0.1 })).toThrow(MathValueError);
    expect(() => discountedCashFlow({ cashFlows: [1], discountRate: -1 })).toThrow(MathValueError);
    expect(() =>
      discountedCashFlow({
        cashFlows: [1],
        discountRate: 0.1,
        terminalGrowthRate: 0.02,
        terminalValue: 10,
      }),
    ).toThrow(MathValueError);
    expect(() =>
      discountedCashFlow({
        cashFlows: [1],
        discountRate: 0.02,
        terminalGrowthRate: 0.02,
      }),
    ).toThrow(MathValueError);
  });
});

describe('NPV / IRR', () => {
  it('计算 NPV 与 IRR', () => {
    expect(netPresentValue([-100, 60, 60], 0.1)).toBeCloseTo(4.1322314, 6);
    expect(internalRateOfReturn([-100, 60, 60])).toBeCloseTo(0.130662, 5);
    expect(internalRateOfReturn([-100, 110])).toBeCloseTo(0.1, 8);
  });

  it('拒绝无解/非法输入', () => {
    expect(() => netPresentValue([], 0.1)).toThrow(MathValueError);
    expect(() => internalRateOfReturn([1, 2, 3])).toThrow(MathValueError);
    expect(() => internalRateOfReturn([-100, 60, 60], 0)).toThrow(MathValueError);
  });
});

describe('每股价值与安全边际', () => {
  it('企业价值转权益、每股价值、安全边际与折现因子', () => {
    expect(equityValueFromEnterpriseValue(1000, 300, 100)).toBe(800);
    expect(perShareValue(800, 100)).toBe(8);
    expect(marginOfSafety(100, 80)).toBeCloseTo(0.2, 10);
    expect(discountFactor(0.1, 1)).toBeCloseTo(0.9090909, 6);
    expect(discountFactor(0.1, 0)).toBe(1);
  });

  it('拒绝非法输入', () => {
    expect(() => perShareValue(100, 0)).toThrow(MathValueError);
    expect(() => marginOfSafety(0, 10)).toThrow(MathValueError);
    expect(() => marginOfSafety(100, -1)).toThrow(MathValueError);
    expect(() => discountFactor(-1, 1)).toThrow(MathValueError);
    expect(() => discountFactor(0.1, -1)).toThrow(MathValueError);
  });
});
