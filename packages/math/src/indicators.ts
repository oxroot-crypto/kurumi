/**
 * @file 技术指标：SMA/EMA/RSI/MACD/布林带/ATR/随机指标/VWAP。
 * @description 纯数学实现，不依赖 provider；所有结果数组按输入索引对齐，warm-up 期为 null。
 * @module math/indicators
 */

import { MathValueError } from './errors.js';

/** OHLCV 点。 */
export interface OhlcvPoint {
  /** 时间（epoch 毫秒）。 */
  readonly timestampMs: number;
  /** 开盘价。 */
  readonly open: number;
  /** 最高价。 */
  readonly high: number;
  /** 最低价。 */
  readonly low: number;
  /** 收盘价。 */
  readonly close: number;
  /** 成交量。 */
  readonly volume: number;
}

/** MACD 指标序列。 */
export interface MacdSeries {
  /** MACD 线。 */
  readonly macd: readonly (number | null)[];
  /** 信号线。 */
  readonly signal: readonly (number | null)[];
  /** 柱状图。 */
  readonly histogram: readonly (number | null)[];
}

/** 布林带序列。 */
export interface BollingerSeries {
  /** 中轨。 */
  readonly middle: readonly (number | null)[];
  /** 上轨。 */
  readonly upper: readonly (number | null)[];
  /** 下轨。 */
  readonly lower: readonly (number | null)[];
  /** 带宽 `(upper-lower)/middle`。 */
  readonly bandwidth: readonly (number | null)[];
  /** `%B = (close-lower)/(upper-lower)`。 */
  readonly percentB: readonly (number | null)[];
}

/** 随机指标序列。 */
export interface StochasticSeries {
  /** %K。 */
  readonly k: readonly (number | null)[];
  /** %D。 */
  readonly d: readonly (number | null)[];
}

/** 技术指标配置。 */
export interface TechnicalIndicatorOptions {
  /** SMA 周期列表，缺省 `[5, 10, 20]`。 */
  readonly smaPeriods?: readonly number[];
  /** EMA 周期列表，缺省 `[12, 26]`。 */
  readonly emaPeriods?: readonly number[];
  /** RSI 周期，缺省 14。 */
  readonly rsiPeriod?: number;
  /** MACD 配置；false 关闭。 */
  readonly macd?:
    false | { readonly fast?: number; readonly slow?: number; readonly signal?: number };
  /** 布林带配置；false 关闭。 */
  readonly bollinger?: false | { readonly period?: number; readonly multiplier?: number };
  /** ATR 周期；false 关闭。 */
  readonly atrPeriod?: false | number;
  /** 随机指标配置；false 关闭。 */
  readonly stochastic?: false | { readonly kPeriod?: number; readonly dPeriod?: number };
  /** 是否计算 VWAP，缺省 true。 */
  readonly vwap?: boolean;
}

/** 技术指标结果。 */
export interface TechnicalIndicatorSeries {
  /** 按周期索引的 SMA。 */
  readonly sma: Readonly<Record<string, readonly (number | null)[]>>;
  /** 按周期索引的 EMA。 */
  readonly ema: Readonly<Record<string, readonly (number | null)[]>>;
  /** RSI。 */
  readonly rsi: readonly (number | null)[];
  /** MACD；关闭时为 null。 */
  readonly macd: MacdSeries | null;
  /** 布林带；关闭时为 null。 */
  readonly bollinger: BollingerSeries | null;
  /** ATR；关闭时为 null。 */
  readonly atr: readonly (number | null)[] | null;
  /** 随机指标；关闭时为 null。 */
  readonly stochastic: StochasticSeries | null;
  /** VWAP；关闭时为 null。 */
  readonly vwap: readonly (number | null)[] | null;
}

/**
 * 简单移动平均。
 *
 * @param values - 数值序列
 * @param period - 周期，> 0
 * @returns 与输入等长的数组；warm-up 为 null
 */
export function sma(values: readonly number[], period: number): readonly (number | null)[] {
  assertSeries(values, 'values');
  assertPeriod(period, 'period');
  const result: Array<number | null> = Array.from({ length: values.length }, () => null);
  let sum = 0;
  for (let index = 0; index < values.length; index += 1) {
    sum += values[index] ?? 0;
    if (index >= period) sum -= values[index - period] ?? 0;
    if (index >= period - 1) result[index] = sum / period;
  }
  return Object.freeze(result);
}

/**
 * 指数移动平均。
 *
 * @param values - 数值序列
 * @param period - 周期，> 0
 * @returns 与输入等长的数组；首个值在 `period-1`，以 SMA 为种子
 */
export function ema(values: readonly number[], period: number): readonly (number | null)[] {
  assertSeries(values, 'values');
  assertPeriod(period, 'period');
  const result: Array<number | null> = Array.from({ length: values.length }, () => null);
  if (values.length < period) return Object.freeze(result);
  let seed = 0;
  for (let index = 0; index < period; index += 1) seed += values[index] ?? 0;
  let previous = seed / period;
  result[period - 1] = previous;
  const multiplier = 2 / (period + 1);
  for (let index = period; index < values.length; index += 1) {
    previous = ((values[index] ?? 0) - previous) * multiplier + previous;
    result[index] = previous;
  }
  return Object.freeze(result);
}

/**
 * RSI（Wilder 平滑）。
 *
 * @param values - 收盘价序列
 * @param period - 周期，缺省 14
 * @returns 与输入等长的数组；warm-up 为 null
 */
export function rsi(values: readonly number[], period = 14): readonly (number | null)[] {
  assertSeries(values, 'values');
  assertPeriod(period, 'period');
  const result: Array<number | null> = Array.from({ length: values.length }, () => null);
  if (values.length <= period) return Object.freeze(result);

  let gain = 0;
  let loss = 0;
  for (let index = 1; index <= period; index += 1) {
    const change = (values[index] ?? 0) - (values[index - 1] ?? 0);
    if (change >= 0) gain += change;
    else loss -= change;
  }
  let averageGain = gain / period;
  let averageLoss = loss / period;
  result[period] = rsiFromAverages(averageGain, averageLoss);

  for (let index = period + 1; index < values.length; index += 1) {
    const change = (values[index] ?? 0) - (values[index - 1] ?? 0);
    const currentGain = change > 0 ? change : 0;
    const currentLoss = change < 0 ? -change : 0;
    averageGain = (averageGain * (period - 1) + currentGain) / period;
    averageLoss = (averageLoss * (period - 1) + currentLoss) / period;
    result[index] = rsiFromAverages(averageGain, averageLoss);
  }
  return Object.freeze(result);
}

/**
 * MACD。
 *
 * @param values - 收盘价序列
 * @param fast - 快线周期，缺省 12
 * @param slow - 慢线周期，缺省 26
 * @param signalPeriod - 信号线周期，缺省 9
 * @returns MACD 线/信号线/柱状图，均与输入等长
 */
export function macd(
  values: readonly number[],
  fast = 12,
  slow = 26,
  signalPeriod = 9,
): MacdSeries {
  assertSeries(values, 'values');
  assertPeriod(fast, 'fast');
  assertPeriod(slow, 'slow');
  assertPeriod(signalPeriod, 'signalPeriod');
  if (fast >= slow) throw new MathValueError('MACD fast 必须小于 slow');

  const fastValues = ema(values, fast);
  const slowValues = ema(values, slow);
  const macdLine: Array<number | null> = Array.from({ length: values.length }, () => null);
  let firstMacdIndex = -1;
  for (let index = 0; index < values.length; index += 1) {
    const fastValue = fastValues[index];
    const slowValue = slowValues[index];
    if (
      fastValue === null ||
      fastValue === undefined ||
      slowValue === null ||
      slowValue === undefined
    ) {
      continue;
    }
    macdLine[index] = fastValue - slowValue;
    if (firstMacdIndex === -1) firstMacdIndex = index;
  }

  const signal: Array<number | null> = Array.from({ length: values.length }, () => null);
  if (firstMacdIndex !== -1) {
    const definedMacd = macdLine.slice(firstMacdIndex).map((value) => value ?? 0);
    const signalValues = ema(definedMacd, signalPeriod);
    for (let offset = 0; offset < signalValues.length; offset += 1) {
      signal[firstMacdIndex + offset] = signalValues[offset] ?? null;
    }
  }
  const histogram = macdLine.map((value, index) => {
    const signalValue = signal[index];
    return value === null || signalValue === null || signalValue === undefined
      ? null
      : value - signalValue;
  });
  return Object.freeze({
    macd: Object.freeze(macdLine),
    signal: Object.freeze(signal),
    histogram: Object.freeze(histogram),
  });
}

/**
 * 布林带。
 *
 * @param values - 收盘价序列
 * @param period - 周期，缺省 20
 * @param multiplier - 标准差倍数，缺省 2
 * @returns 中/上/下轨、带宽与 %B
 */
export function bollingerBands(
  values: readonly number[],
  period = 20,
  multiplier = 2,
): BollingerSeries {
  assertSeries(values, 'values');
  assertPeriod(period, 'period');
  if (!Number.isFinite(multiplier) || multiplier <= 0) {
    throw new MathValueError('multiplier 必须是正有限数');
  }
  const middle = sma(values, period);
  const upper: Array<number | null> = Array.from({ length: values.length }, () => null);
  const lower: Array<number | null> = Array.from({ length: values.length }, () => null);
  const bandwidth: Array<number | null> = Array.from({ length: values.length }, () => null);
  const percentB: Array<number | null> = Array.from({ length: values.length }, () => null);

  for (let index = period - 1; index < values.length; index += 1) {
    const mean = middle[index];
    if (mean === null || mean === undefined) continue;
    let variance = 0;
    for (let offset = index - period + 1; offset <= index; offset += 1) {
      const difference = (values[offset] ?? 0) - mean;
      variance += difference * difference;
    }
    const deviation = Math.sqrt(variance / period);
    const upperValue = mean + multiplier * deviation;
    const lowerValue = mean - multiplier * deviation;
    upper[index] = upperValue;
    lower[index] = lowerValue;
    bandwidth[index] = mean === 0 ? null : (upperValue - lowerValue) / mean;
    const range = upperValue - lowerValue;
    percentB[index] = range === 0 ? null : ((values[index] ?? 0) - lowerValue) / range;
  }

  return Object.freeze({
    middle,
    upper: Object.freeze(upper),
    lower: Object.freeze(lower),
    bandwidth: Object.freeze(bandwidth),
    percentB: Object.freeze(percentB),
  });
}

/**
 * 真实波幅。
 *
 * @param highs - 最高价序列
 * @param lows - 最低价序列
 * @param closes - 收盘价序列
 * @returns 与输入等长的真实波幅
 */
export function trueRange(
  highs: readonly number[],
  lows: readonly number[],
  closes: readonly number[],
): readonly number[] {
  assertAligned(highs, lows, closes);
  const result: number[] = [];
  for (let index = 0; index < highs.length; index += 1) {
    const high = highs[index] ?? 0;
    const low = lows[index] ?? 0;
    if (index === 0) {
      result.push(high - low);
      continue;
    }
    const previousClose = closes[index - 1] ?? 0;
    result.push(
      Math.max(high - low, Math.abs(high - previousClose), Math.abs(low - previousClose)),
    );
  }
  return Object.freeze(result);
}

/**
 * ATR（Wilder 平滑）。
 *
 * @param highs - 最高价序列
 * @param lows - 最低价序列
 * @param closes - 收盘价序列
 * @param period - 周期，缺省 14
 * @returns 与输入等长的 ATR；warm-up 为 null
 */
export function atr(
  highs: readonly number[],
  lows: readonly number[],
  closes: readonly number[],
  period = 14,
): readonly (number | null)[] {
  assertPeriod(period, 'period');
  const ranges = trueRange(highs, lows, closes);
  const result: Array<number | null> = Array.from({ length: ranges.length }, () => null);
  if (ranges.length < period) return Object.freeze(result);
  let seed = 0;
  for (let index = 0; index < period; index += 1) seed += ranges[index] ?? 0;
  let previous = seed / period;
  result[period - 1] = previous;
  for (let index = period; index < ranges.length; index += 1) {
    previous = (previous * (period - 1) + (ranges[index] ?? 0)) / period;
    result[index] = previous;
  }
  return Object.freeze(result);
}

/**
 * 随机指标 %K/%D。
 *
 * @param highs - 最高价序列
 * @param lows - 最低价序列
 * @param closes - 收盘价序列
 * @param kPeriod - %K 周期，缺省 14
 * @param dPeriod - %D 周期，缺省 3
 * @returns %K/%D，与输入等长
 */
export function stochastic(
  highs: readonly number[],
  lows: readonly number[],
  closes: readonly number[],
  kPeriod = 14,
  dPeriod = 3,
): StochasticSeries {
  assertAligned(highs, lows, closes);
  assertPeriod(kPeriod, 'kPeriod');
  assertPeriod(dPeriod, 'dPeriod');
  const k: Array<number | null> = Array.from({ length: closes.length }, () => null);
  for (let index = kPeriod - 1; index < closes.length; index += 1) {
    let highest = Number.NEGATIVE_INFINITY;
    let lowest = Number.POSITIVE_INFINITY;
    for (let offset = index - kPeriod + 1; offset <= index; offset += 1) {
      highest = Math.max(highest, highs[offset] ?? highest);
      lowest = Math.min(lowest, lows[offset] ?? lowest);
    }
    const range = highest - lowest;
    k[index] = range === 0 ? 50 : (((closes[index] ?? 0) - lowest) / range) * 100;
  }
  const firstK = k.findIndex((value) => value !== null);
  const d: Array<number | null> = Array.from({ length: closes.length }, () => null);
  if (firstK !== -1) {
    const definedK = k.slice(firstK).map((value) => value ?? 0);
    const dValues = sma(definedK, dPeriod);
    for (let offset = 0; offset < dValues.length; offset += 1) {
      d[firstK + offset] = dValues[offset] ?? null;
    }
  }
  return Object.freeze({ k: Object.freeze(k), d: Object.freeze(d) });
}

/**
 * VWAP（累计成交量加权平均价）。
 *
 * @param points - OHLCV 序列
 * @returns 与输入等长的 VWAP；累计成交量为 0 时为 null
 */
export function vwap(points: readonly OhlcvPoint[]): readonly (number | null)[] {
  assertOhlcv(points);
  const result: Array<number | null> = [];
  let cumulativeValue = 0;
  let cumulativeVolume = 0;
  for (const point of points) {
    const typical = (point.high + point.low + point.close) / 3;
    cumulativeValue += typical * point.volume;
    cumulativeVolume += point.volume;
    result.push(cumulativeVolume === 0 ? null : cumulativeValue / cumulativeVolume);
  }
  return Object.freeze(result);
}

/**
 * 计算一组常用技术指标。
 *
 * @param points - OHLCV 序列
 * @param options - 周期与开关
 * @returns 按输入索引对齐的指标序列
 */
export function computeTechnicalIndicators(
  points: readonly OhlcvPoint[],
  options: TechnicalIndicatorOptions = {},
): TechnicalIndicatorSeries {
  assertOhlcv(points);
  const closes = points.map((point) => point.close);
  const highs = points.map((point) => point.high);
  const lows = points.map((point) => point.low);

  const smaPeriods = options.smaPeriods ?? [5, 10, 20];
  const emaPeriods = options.emaPeriods ?? [12, 26];
  for (const period of [...smaPeriods, ...emaPeriods]) assertPeriod(period, 'period');
  const smaValues: Record<string, readonly (number | null)[]> = {};
  for (const period of smaPeriods) smaValues[String(period)] = sma(closes, period);
  const emaValues: Record<string, readonly (number | null)[]> = {};
  for (const period of emaPeriods) emaValues[String(period)] = ema(closes, period);

  const rsiPeriod = options.rsiPeriod ?? 14;
  assertPeriod(rsiPeriod, 'rsiPeriod');
  const macdOption = options.macd ?? {};
  const bollingerOption = options.bollinger ?? {};
  const stochasticOption = options.stochastic ?? {};
  const atrOption = options.atrPeriod ?? 14;

  return Object.freeze({
    sma: Object.freeze(smaValues),
    ema: Object.freeze(emaValues),
    rsi: rsi(closes, rsiPeriod),
    macd:
      macdOption === false
        ? null
        : macd(closes, macdOption.fast ?? 12, macdOption.slow ?? 26, macdOption.signal ?? 9),
    bollinger:
      bollingerOption === false
        ? null
        : bollingerBands(closes, bollingerOption.period ?? 20, bollingerOption.multiplier ?? 2),
    atr: atrOption === false ? null : atr(highs, lows, closes, atrOption),
    stochastic:
      stochasticOption === false
        ? null
        : stochastic(
            highs,
            lows,
            closes,
            stochasticOption.kPeriod ?? 14,
            stochasticOption.dPeriod ?? 3,
          ),
    vwap: options.vwap === false ? null : vwap(points),
  });
}

/** RSI 计算。 */
function rsiFromAverages(averageGain: number, averageLoss: number): number {
  if (averageLoss === 0) return averageGain === 0 ? 50 : 100;
  if (averageGain === 0) return 0;
  const relativeStrength = averageGain / averageLoss;
  return 100 - 100 / (1 + relativeStrength);
}

/** 校验序列。 */
function assertSeries(values: readonly number[], path: string): void {
  if (!Array.isArray(values) || values.length === 0) {
    throw new MathValueError(`${path} 不能为空`);
  }
  for (let index = 0; index < values.length; index += 1) {
    const value = values[index];
    if (value === undefined || !Number.isFinite(value)) {
      throw new MathValueError(`${path}[${index}] 必须是有限数值`);
    }
  }
}

/** 校验周期。 */
function assertPeriod(period: number, path: string): void {
  if (!Number.isSafeInteger(period) || period <= 0) {
    throw new MathValueError(`${path} 必须是正安全整数（实际值: ${String(period)}）`);
  }
}

/** 校验等长序列。 */
function assertAligned(
  highs: readonly number[],
  lows: readonly number[],
  closes: readonly number[],
): void {
  assertSeries(highs, 'highs');
  assertSeries(lows, 'lows');
  assertSeries(closes, 'closes');
  if (highs.length !== lows.length || highs.length !== closes.length) {
    throw new MathValueError('highs/lows/closes 长度必须一致');
  }
}

/** 校验 OHLCV。 */
function assertOhlcv(points: readonly OhlcvPoint[]): void {
  if (!Array.isArray(points) || points.length === 0) {
    throw new MathValueError('points 不能为空');
  }
  for (let index = 0; index < points.length; index += 1) {
    const point = points[index];
    if (point === undefined) throw new MathValueError(`points[${index}] 缺失`);
    for (const [key, value] of [
      ['open', point.open],
      ['high', point.high],
      ['low', point.low],
      ['close', point.close],
      ['volume', point.volume],
    ] as const) {
      if (!Number.isFinite(value)) {
        throw new MathValueError(`points[${index}].${key} 必须是有限数值`);
      }
    }
    if (point.volume < 0) throw new MathValueError(`points[${index}].volume 不能为负`);
    if (point.high < point.low) throw new MathValueError(`points[${index}].high 不能低于 low`);
    if (point.high < point.open || point.high < point.close) {
      throw new MathValueError(`points[${index}].high 必须不低于 open/close`);
    }
    if (point.low > point.open || point.low > point.close) {
      throw new MathValueError(`points[${index}].low 必须不高于 open/close`);
    }
  }
}
