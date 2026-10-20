/**
 * @file K 线技术指标适配：把 finance `Bar` 转为 math 的 OHLCV 并计算指标。
 * @description 与 provider 无关；任何来源的 Bar 都可以使用。
 * @module finance/indicators
 */

import {
  computeTechnicalIndicators,
  type OhlcvPoint,
  type TechnicalIndicatorOptions,
  type TechnicalIndicatorSeries,
} from '@kurumi/math';

import type { Bar } from './market-data.js';

/** 带时间戳的 K 线指标结果。 */
export interface BarIndicatorSeries extends TechnicalIndicatorSeries {
  /** 与指标数组逐一对应的 K 线开盘时间。 */
  readonly timestamps: readonly number[];
}

/**
 * 计算 K 线技术指标。
 *
 * @param bars - K 线序列，按时间升序
 * @param options - 指标周期与开关
 * @returns 指标数组与 `timestamps`；数组索引与 bars 一致
 * @throws {MathValueError} bars 为空或指标参数非法
 *
 * @example
 * ```ts
 * const indicators = computeBarIndicators(bars);
 * indicators.macd?.histogram.at(-1);
 * ```
 */
export function computeBarIndicators(
  bars: readonly Bar[],
  options?: TechnicalIndicatorOptions,
): BarIndicatorSeries {
  const points: OhlcvPoint[] = bars.map((bar) => ({
    timestampMs: bar.openTimeMs,
    open: bar.open.toNumber(),
    high: bar.high.toNumber(),
    low: bar.low.toNumber(),
    close: bar.close.toNumber(),
    volume: bar.volume.toNumber(),
  }));
  const series = computeTechnicalIndicators(points, options);
  return Object.freeze({
    timestamps: Object.freeze(bars.map((bar) => bar.openTimeMs)),
    ...series,
  });
}
