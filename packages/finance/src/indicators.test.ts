/**
 * @file K 线指标适配测试。
 * @description 验证 finance Bar -> math OHLCV 的映射、timestamps 对齐与指标结果。
 * @module finance/indicators.test
 */

import { describe, expect, it } from 'vitest';

import { Decimal } from '@kurumi/math';

import { computeBarIndicators } from './indicators.js';
import { BarInterval, createBar } from './market-data.js';
import { T0, btcUsdt } from './fixtures.test-utils.js';

const DAY_MS = 86_400_000;

function bars(count = 30) {
  const instrument = btcUsdt();
  return Array.from({ length: count }, (_, index) => {
    const close = 100 + index * 0.5;
    const openTimeMs = T0 + index * DAY_MS;
    return createBar({
      instrument,
      interval: BarInterval.Day1,
      openTimeMs,
      closeTimeMs: openTimeMs + DAY_MS,
      open: Decimal.parse(String(close - 0.25)),
      high: Decimal.parse(String(close + 0.5)),
      low: Decimal.parse(String(close - 0.5)),
      close: Decimal.parse(String(close)),
      volume: Decimal.parse('10'),
    });
  });
}

describe('computeBarIndicators', () => {
  it('映射 Bar 并计算默认指标', () => {
    const source = bars();
    const result = computeBarIndicators(source);
    expect(result.timestamps).toEqual(source.map((bar) => bar.openTimeMs));
    expect(result.sma['5']).toHaveLength(30);
    expect(result.ema['12']).toHaveLength(30);
    expect(result.rsi).toHaveLength(30);
    expect(result.macd?.macd).toHaveLength(30);
    expect(result.bollinger?.middle).toHaveLength(30);
    expect(result.atr).toHaveLength(30);
    expect(result.stochastic?.k).toHaveLength(30);
    expect(result.vwap).toHaveLength(30);
  });

  it('支持自定义周期与关闭指标', () => {
    const result = computeBarIndicators(bars(), {
      smaPeriods: [3],
      emaPeriods: [5],
      rsiPeriod: 5,
      macd: false,
      bollinger: false,
      atrPeriod: false,
      stochastic: false,
      vwap: false,
    });
    expect(Object.keys(result.sma)).toEqual(['3']);
    expect(result.macd).toBeNull();
    expect(result.bollinger).toBeNull();
  });
});
