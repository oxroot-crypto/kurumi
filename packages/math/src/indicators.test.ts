/**
 * @file 技术指标单元测试。
 * @description 覆盖 SMA/EMA/RSI/MACD/布林带/ATR/随机指标/VWAP 的数值、warm-up 与边界。
 * @module math/indicators.test
 */

import { describe, expect, it } from 'vitest';

import { MathValueError } from './errors.js';
import {
  atr,
  bollingerBands,
  computeTechnicalIndicators,
  ema,
  macd,
  rsi,
  sma,
  stochastic,
  trueRange,
  vwap,
  type OhlcvPoint,
} from './indicators.js';

describe('SMA / EMA', () => {
  it('SMA warm-up 与窗口均值', () => {
    expect(sma([1, 2, 3, 4, 5], 3)).toEqual([null, null, 2, 3, 4]);
    expect(sma([1, 2], 3)).toEqual([null, null]);
    expect(() => sma([], 3)).toThrow(MathValueError);
    expect(() => sma([1], 0)).toThrow(MathValueError);
  });

  it('EMA 以 SMA 为种子', () => {
    expect(ema([1, 2, 3, 4, 5], 3)).toEqual([null, null, 2, 3, 4]);
    expect(ema([1, 2], 3)).toEqual([null, null]);
  });
});

describe('RSI / MACD', () => {
  it('单调上涨/下跌的 RSI 边界', () => {
    const rising = rsi([1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 3);
    expect(rising[3]).toBe(100);
    expect(rising[9]).toBe(100);
    const falling = rsi([10, 9, 8, 7, 6, 5, 4, 3, 2, 1], 3);
    expect(falling[3]).toBe(0);
    const flat = rsi([1, 1, 1, 1, 1], 3);
    expect(flat[3]).toBe(50);
    expect(rsi([1, 2], 14).every((value) => value === null)).toBe(true);
  });

  it('MACD 对齐、信号线与柱状图', () => {
    const values = Array.from({ length: 40 }, (_, index) => index + 1);
    const result = macd(values, 3, 6, 3);
    expect(result.macd).toHaveLength(40);
    expect(result.macd[4]).toBeNull();
    expect(result.macd[5]).not.toBeNull();
    const index = result.histogram.findIndex((value) => value !== null);
    expect(index).toBeGreaterThanOrEqual(0);
    expect(result.histogram[index]).toBeCloseTo(
      (result.macd[index] ?? 0) - (result.signal[index] ?? 0),
      10,
    );
    expect(() => macd(values, 6, 6)).toThrow(MathValueError);
  });
});

describe('布林带 / ATR / 随机指标 / VWAP', () => {
  it('布林带数值', () => {
    const result = bollingerBands([1, 2, 3, 4, 5], 3, 2);
    expect(result.middle).toEqual([null, null, 2, 3, 4]);
    expect(result.upper[2]).toBeCloseTo(3.632993, 5);
    expect(result.lower[2]).toBeCloseTo(0.367007, 5);
    expect(result.bandwidth[2]).toBeCloseTo(1.632993, 5);
    expect(result.percentB[2]).toBeCloseTo(0.806186, 5);
    expect(() => bollingerBands([1], 1, 0)).toThrow(MathValueError);
  });

  it('真实波幅与 ATR', () => {
    expect(trueRange([10, 12, 13], [8, 9, 11], [9, 11, 12])).toEqual([2, 3, 2]);
    const result = atr([10, 12, 13], [8, 9, 11], [9, 11, 12], 2);
    expect(result).toEqual([null, 2.5, 2.25]);
    expect(() => trueRange([1], [1, 2], [1])).toThrow(MathValueError);
  });

  it('随机指标与 VWAP', () => {
    const stochasticResult = stochastic([10, 11, 12, 13], [8, 9, 10, 11], [9, 10, 11, 12], 3, 2);
    expect(stochasticResult.k[2]).toBeCloseTo(75, 10);
    expect(stochasticResult.k[3]).toBeCloseTo(75, 10);
    expect(stochasticResult.d[3]).toBeCloseTo(75, 10);

    const points: OhlcvPoint[] = [
      { timestampMs: 1, open: 9, high: 10, low: 8, close: 9, volume: 1 },
      { timestampMs: 2, open: 10, high: 12, low: 10, close: 11, volume: 3 },
    ];
    const result = vwap(points);
    expect(result[0]).toBeCloseTo(9, 10);
    expect(result[1]).toBeCloseTo((9 * 1 + 11 * 3) / 4, 10);
    expect(() => vwap([])).toThrow(MathValueError);
  });
});

describe('computeTechnicalIndicators', () => {
  const points: OhlcvPoint[] = Array.from({ length: 30 }, (_, index) => {
    const close = 100 + index;
    return {
      timestampMs: index * 86_400_000,
      open: close - 0.5,
      high: close + 1,
      low: close - 1,
      close,
      volume: 10 + index,
    };
  });

  it('返回默认指标且长度对齐', () => {
    const result = computeTechnicalIndicators(points);
    expect(Object.keys(result.sma)).toEqual(['5', '10', '20']);
    expect(Object.keys(result.ema)).toEqual(['12', '26']);
    expect(result.sma['5']).toHaveLength(30);
    expect(result.rsi).toHaveLength(30);
    expect(result.macd?.macd).toHaveLength(30);
    expect(result.bollinger?.upper).toHaveLength(30);
    expect(result.atr).toHaveLength(30);
    expect(result.stochastic?.k).toHaveLength(30);
    expect(result.vwap).toHaveLength(30);
  });

  it('支持关闭可选指标', () => {
    const result = computeTechnicalIndicators(points, {
      smaPeriods: [3],
      emaPeriods: [5],
      macd: false,
      bollinger: false,
      atrPeriod: false,
      stochastic: false,
      vwap: false,
    });
    expect(result.macd).toBeNull();
    expect(result.bollinger).toBeNull();
    expect(result.atr).toBeNull();
    expect(result.stochastic).toBeNull();
    expect(result.vwap).toBeNull();
  });

  it('拒绝非法 OHLCV', () => {
    expect(() => computeTechnicalIndicators([])).toThrow(MathValueError);
    expect(() =>
      computeTechnicalIndicators([
        { timestampMs: 1, open: 10, high: 9, low: 8, close: 9, volume: 1 },
      ]),
    ).toThrow(MathValueError);
    expect(() =>
      computeTechnicalIndicators([
        { timestampMs: 1, open: 9, high: 10, low: 8, close: 9, volume: -1 },
      ]),
    ).toThrow(MathValueError);
  });
});
