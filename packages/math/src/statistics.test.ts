/**
 * @file 绩效与风险统计单元测试。
 * @description 覆盖 FC-PERF-* 的收益、年化、回撤、Beta/Alpha、VaR/CVaR 与边界。
 * @module math/statistics.test
 */

import { describe, expect, it } from 'vitest';

import { MathValueError } from './errors.js';
import {
  alpha,
  annualizedReturn,
  annualizedVolatility,
  beta,
  calmarRatio,
  compoundAnnualGrowthRate,
  correlation,
  covariance,
  cumulativeReturn,
  drawdownSeries,
  expectedShortfall,
  historicalVaR,
  informationRatio,
  logReturns,
  maxDrawdown,
  parametricVaR,
  sharpeRatio,
  simpleReturns,
  sortinoRatio,
  trackingError,
  variance,
} from './statistics.js';

const prices = [100, 110, 99, 120];
const returns = simpleReturns(prices);

describe('收益与年化', () => {
  it('简单/对数收益、累计收益与年化收益', () => {
    expect(returns[0]).toBeCloseTo(0.1, 10);
    expect(returns[1]).toBeCloseTo(-0.1, 10);
    expect(returns[2]).toBeCloseTo(0.2121212121, 8);
    const logs = logReturns(prices);
    expect(logs[0]).toBeCloseTo(Math.log(1.1), 10);
    expect(cumulativeReturn(returns)).toBeCloseTo(0.2, 10);
    expect(annualizedReturn(returns, 12)).toBeCloseTo(1.0736, 8);
    expect(compoundAnnualGrowthRate(100, 200, 2)).toBeCloseTo(Math.SQRT2 - 1, 10);
  });

  it('拒绝非法价格与收益率', () => {
    expect(() => simpleReturns([100])).toThrow(MathValueError);
    expect(() => simpleReturns([100, 0])).toThrow(MathValueError);
    expect(() => logReturns([100, -1])).toThrow(MathValueError);
    expect(() => cumulativeReturn([-1])).toThrow(MathValueError);
    expect(() => annualizedReturn([], 12)).toThrow(MathValueError);
    expect(() => annualizedReturn([0.1], 0)).toThrow(MathValueError);
  });
});

describe('波动率与风险调整收益', () => {
  it('年化波动率、Sharpe、Sortino', () => {
    expect(annualizedVolatility(returns, 12)).toBeCloseTo(0.5477057919, 8);
    expect(sharpeRatio(returns, 0.02, 12)).toBeCloseTo(1.9236605044, 8);
    expect(sortinoRatio(returns, 0.02, 12)).toBeCloseTo(5.268, 8);
    expect(variance([0.1, 0.2, 0.3])).toBeCloseTo(0.01, 10);
  });

  it('零波动/零下行偏差失败', () => {
    expect(() => sharpeRatio([0.1, 0.1, 0.1], 0, 12)).toThrow(MathValueError);
    expect(() => sortinoRatio([0.1, 0.2, 0.3], 0, 12)).toThrow(MathValueError);
  });
});

describe('回撤', () => {
  it('最大回撤、回撤序列与 Calmar', () => {
    const drawdown = maxDrawdown(prices);
    expect(drawdown.maxDrawdown).toBeCloseTo(-0.1, 10);
    expect(drawdown.peakIndex).toBe(1);
    expect(drawdown.troughIndex).toBe(2);
    const series = drawdownSeries(prices);
    expect(series[0]).toBe(0);
    expect(series[1]).toBe(0);
    expect(series[2]).toBeCloseTo(-0.1, 10);
    expect(series[3]).toBe(0);
    expect(calmarRatio(returns, 12)).toBeCloseTo(10.736, 6);
    expect(maxDrawdown([1, 2, 3]).maxDrawdown).toBe(0);
  });

  it('拒绝非法价格序列与零回撤 Calmar', () => {
    expect(() => maxDrawdown([1])).toThrow(MathValueError);
    expect(() => drawdownSeries([1, 0])).toThrow(MathValueError);
    expect(() => calmarRatio([0.1, 0.1], 12)).toThrow(MathValueError);
  });
});

describe('Beta / Alpha / 跟踪误差', () => {
  const benchmark = [0.05, 0.1, 0.15];
  const asset = [0.1, 0.2, 0.3];

  it('协方差、Beta、相关系数、Alpha 与跟踪误差', () => {
    expect(covariance(asset, benchmark)).toBeCloseTo(0.005, 10);
    expect(beta(asset, benchmark)).toBeCloseTo(2, 10);
    expect(correlation(asset, benchmark)).toBeCloseTo(1, 10);
    expect(alpha(benchmark, benchmark, 0.02, 12)).toBeCloseTo(0, 10);
    expect(trackingError(benchmark, benchmark, 12)).toBeCloseTo(0, 10);
    expect(correlation(returns, returns)).toBeCloseTo(1, 10);
  });

  it('零方差与长度不一致失败', () => {
    expect(() => correlation([1, 1], [1, 2])).toThrow(MathValueError);
    expect(() => beta([1, 2], [1, 1])).toThrow(MathValueError);
    expect(() => covariance([1, 2], [1])).toThrow(MathValueError);
    expect(() => informationRatio(benchmark, benchmark, 12)).toThrow(MathValueError);
  });
});

describe('VaR / CVaR', () => {
  const sample = [-0.1, -0.05, 0, 0.02, 0.03];

  it('历史 VaR、参数 VaR 与期望损失', () => {
    expect(historicalVaR(sample, 0.8)).toBeCloseTo(0.05, 10);
    expect(historicalVaR(sample, 0.95)).toBeCloseTo(0.1, 10);
    expect(parametricVaR([-0.5, -0.4, -0.3, -0.2, -0.1], 0.95)).toBeCloseTo(0.5600741939377787, 9);
    // 正收益均值时左尾仍可能为损失，不能被 max(0) 吞掉。
    expect(parametricVaR([0.05, 0.1, -0.1, 0.08, 0.02], 0.95)).toBeGreaterThan(0);
    expect(expectedShortfall(sample, 0.8)).toBeCloseTo(0.1, 10);
    expect(historicalVaR([0.01, 0.02], 0.95)).toBe(0);
  });

  it('拒绝非法置信度与序列', () => {
    expect(() => historicalVaR(sample, 0)).toThrow(MathValueError);
    expect(() => historicalVaR(sample, 1)).toThrow(MathValueError);
    expect(() => parametricVaR([1], 0.95)).toThrow(MathValueError);
    expect(() => expectedShortfall([0.1, Number.NaN], 0.95)).toThrow(MathValueError);
  });
});
