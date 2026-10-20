/**
 * @file 统计与风险度量：收益、年化、波动率、Sharpe/Sortino、回撤、Beta/Alpha、VaR/CVaR。
 * @description 统计层使用 Float64；输入必须是有限数值，输出为模型/分析数值，不直接进入账务。
 * @module math/statistics
 */

import { MathValueError } from './errors.js';

/** 浮点零判定阈值。 */
const ZERO_TOLERANCE = 1e-12;

/** 最大回撤结果。 */
export interface MaxDrawdownResult {
  /** 最大回撤（<= 0）。 */
  readonly maxDrawdown: number;
  /** 峰值索引。 */
  readonly peakIndex: number;
  /** 谷值索引。 */
  readonly troughIndex: number;
}

/**
 * 简单收益率序列。
 *
 * @param prices - 价格序列，长度 >= 2 且全部 > 0
 * @returns `p_t/p_{t-1} - 1`
 * @throws {MathValueError} 序列非法
 */
export function simpleReturns(prices: readonly number[]): number[] {
  assertPrices(prices);
  const returns: number[] = [];
  for (let index = 1; index < prices.length; index += 1) {
    const previous = prices[index - 1];
    const current = prices[index];
    if (previous === undefined || current === undefined) continue;
    returns.push(current / previous - 1);
  }
  return returns;
}

/**
 * 对数收益率序列。
 *
 * @param prices - 价格序列，长度 >= 2 且全部 > 0
 * @returns `ln(p_t/p_{t-1})`
 * @throws {MathValueError} 序列非法
 */
export function logReturns(prices: readonly number[]): number[] {
  assertPrices(prices);
  const returns: number[] = [];
  for (let index = 1; index < prices.length; index += 1) {
    const previous = prices[index - 1];
    const current = prices[index];
    if (previous === undefined || current === undefined) continue;
    returns.push(Math.log(current / previous));
  }
  return returns;
}

/**
 * 累计收益率。
 *
 * @param returns - 收益率序列，每项必须 > -1
 * @returns `Π(1+r)-1`
 * @throws {MathValueError} 序列非法
 */
export function cumulativeReturn(returns: readonly number[]): number {
  assertReturns(returns, true);
  let product = 1;
  for (const value of returns) product *= 1 + value;
  return product - 1;
}

/**
 * 年化收益率（几何）。
 *
 * @param returns - 收益率序列，每项必须 > -1
 * @param periodsPerYear - 每年期数，> 0
 * @returns 几何年化收益
 * @throws {MathValueError} 序列非法或年化后无意义
 */
export function annualizedReturn(returns: readonly number[], periodsPerYear: number): number {
  assertReturns(returns, true);
  assertPositiveFinite(periodsPerYear, 'periodsPerYear');
  let product = 1;
  for (const value of returns) product *= 1 + value;
  if (product <= 0) {
    throw new MathValueError('累计收益非正，无法计算几何年化收益');
  }
  return Math.pow(product, periodsPerYear / returns.length) - 1;
}

/**
 * 年化波动率（样本标准差）。
 *
 * @param returns - 收益率序列，长度 >= 2
 * @param periodsPerYear - 每年期数，> 0
 * @returns `stddev(returns) * sqrt(periodsPerYear)`
 * @throws {MathValueError} 序列非法
 */
export function annualizedVolatility(returns: readonly number[], periodsPerYear: number): number {
  assertReturns(returns, false);
  assertPositiveFinite(periodsPerYear, 'periodsPerYear');
  return sampleStdDev(returns) * Math.sqrt(periodsPerYear);
}

/**
 * Sharpe 比率。
 *
 * @param returns - 收益率序列
 * @param riskFreeRate - 年化无风险利率
 * @param periodsPerYear - 每年期数
 * @returns 年化超额收益 / 年化波动率
 * @throws {MathValueError} 波动率为 0 或参数非法
 */
export function sharpeRatio(
  returns: readonly number[],
  riskFreeRate: number,
  periodsPerYear: number,
): number {
  assertFinite(riskFreeRate, 'riskFreeRate');
  assertPositiveFinite(periodsPerYear, 'periodsPerYear');
  const excess = annualizedReturn(returns, periodsPerYear) - riskFreeRate;
  const volatility = annualizedVolatility(returns, periodsPerYear);
  if (isNearZero(volatility)) {
    throw new MathValueError('波动率为 0，无法计算 Sharpe 比率');
  }
  return excess / volatility;
}

/**
 * Sortino 比率。
 *
 * @param returns - 收益率序列
 * @param riskFreeRate - 年化无风险利率
 * @param periodsPerYear - 每年期数
 * @returns 年化超额收益 / 年化下行偏差
 * @throws {MathValueError} 下行偏差为 0 或参数非法
 */
export function sortinoRatio(
  returns: readonly number[],
  riskFreeRate: number,
  periodsPerYear: number,
): number {
  assertFinite(riskFreeRate, 'riskFreeRate');
  assertPositiveFinite(periodsPerYear, 'periodsPerYear');
  const excess = annualizedReturn(returns, periodsPerYear) - riskFreeRate;
  let downsideSquares = 0;
  for (const value of returns) {
    const downside = Math.min(value, 0);
    downsideSquares += downside * downside;
  }
  const downsideDeviation = Math.sqrt(downsideSquares / returns.length) * Math.sqrt(periodsPerYear);
  if (isNearZero(downsideDeviation)) {
    throw new MathValueError('下行偏差为 0，无法计算 Sortino 比率');
  }
  return excess / downsideDeviation;
}

/**
 * 最大回撤。
 *
 * @param prices - 价格序列，长度 >= 2 且全部 > 0
 * @returns 最大回撤（<= 0）及峰/谷索引
 * @throws {MathValueError} 序列非法
 */
export function maxDrawdown(prices: readonly number[]): MaxDrawdownResult {
  assertPrices(prices);
  let peak = prices[0] ?? 0;
  let peakIndex = 0;
  let maxValue = 0;
  let bestPeakIndex = 0;
  let troughIndex = 0;
  for (let index = 1; index < prices.length; index += 1) {
    const price = prices[index];
    if (price === undefined) continue;
    if (price > peak) {
      peak = price;
      peakIndex = index;
      continue;
    }
    const drawdown = price / peak - 1;
    if (drawdown < maxValue) {
      maxValue = drawdown;
      bestPeakIndex = peakIndex;
      troughIndex = index;
    }
  }
  return Object.freeze({
    maxDrawdown: maxValue,
    peakIndex: bestPeakIndex,
    troughIndex,
  });
}

/**
 * 回撤序列。
 *
 * @param prices - 价格序列，长度 >= 2 且全部 > 0
 * @returns 每个时点相对历史峰值的回撤（<= 0）
 */
export function drawdownSeries(prices: readonly number[]): number[] {
  assertPrices(prices);
  let peak = prices[0] ?? 0;
  const result: number[] = [];
  for (const price of prices) {
    if (price > peak) peak = price;
    result.push(price / peak - 1);
  }
  return result;
}

/**
 * Calmar 比率。
 *
 * @param returns - 收益率序列
 * @param periodsPerYear - 每年期数
 * @returns 年化收益 / |最大回撤|
 * @throws {MathValueError} 最大回撤为 0 或参数非法
 */
export function calmarRatio(returns: readonly number[], periodsPerYear: number): number {
  const annual = annualizedReturn(returns, periodsPerYear);
  const equityCurve: number[] = [1];
  let value = 1;
  for (const item of returns) {
    value *= 1 + item;
    equityCurve.push(value);
  }
  const drawdown = maxDrawdown(equityCurve).maxDrawdown;
  if (isNearZero(drawdown)) {
    throw new MathValueError('最大回撤为 0，无法计算 Calmar 比率');
  }
  return annual / Math.abs(drawdown);
}

/**
 * 样本协方差。
 *
 * @param left - 左序列，长度 >= 2
 * @param right - 右序列，长度必须与 left 相同
 * @returns 样本协方差
 */
export function covariance(left: readonly number[], right: readonly number[]): number {
  assertPairedSeries(left, right);
  const leftMean = mean(left);
  const rightMean = mean(right);
  let total = 0;
  for (let index = 0; index < left.length; index += 1) {
    const leftValue = left[index];
    const rightValue = right[index];
    if (leftValue === undefined || rightValue === undefined) continue;
    total += (leftValue - leftMean) * (rightValue - rightMean);
  }
  return total / (left.length - 1);
}

/**
 * 样本方差。
 *
 * @param values - 数值序列，长度 >= 2
 * @returns 样本方差
 */
export function variance(values: readonly number[]): number {
  assertReturns(values, false);
  return sampleVariance(values);
}

/**
 * 相关系数。
 *
 * @param left - 左序列，长度 >= 2
 * @param right - 右序列，长度必须相同
 * @returns `[-1, 1]` 内的相关系数
 * @throws {MathValueError} 任一序列方差为 0
 */
export function correlation(left: readonly number[], right: readonly number[]): number {
  const leftVariance = variance(left);
  const rightVariance = variance(right);
  if (isNearZero(leftVariance) || isNearZero(rightVariance)) {
    throw new MathValueError('任一序列方差为 0，无法计算相关系数');
  }
  return covariance(left, right) / Math.sqrt(leftVariance * rightVariance);
}

/**
 * Beta。
 *
 * @param assetReturns - 资产收益率
 * @param benchmarkReturns - 基准收益率
 * @returns `Cov(asset, benchmark) / Var(benchmark)`
 * @throws {MathValueError} 基准方差为 0 或序列非法
 */
export function beta(assetReturns: readonly number[], benchmarkReturns: readonly number[]): number {
  assertPairedSeries(assetReturns, benchmarkReturns);
  const benchmarkVariance = sampleVariance(benchmarkReturns);
  if (isNearZero(benchmarkVariance)) {
    throw new MathValueError('基准方差为 0，无法计算 Beta');
  }
  return covariance(assetReturns, benchmarkReturns) / benchmarkVariance;
}

/**
 * Jensen Alpha（年化）。
 *
 * @param assetReturns - 资产收益率
 * @param benchmarkReturns - 基准收益率
 * @param riskFreeRate - 年化无风险利率
 * @param periodsPerYear - 每年期数
 * @returns 年化 Jensen Alpha
 * @throws {MathValueError} 参数非法
 */
export function alpha(
  assetReturns: readonly number[],
  benchmarkReturns: readonly number[],
  riskFreeRate: number,
  periodsPerYear: number,
): number {
  assertPairedSeries(assetReturns, benchmarkReturns);
  assertFinite(riskFreeRate, 'riskFreeRate');
  assertPositiveFinite(periodsPerYear, 'periodsPerYear');
  const periodRiskFree = riskFreeRate / periodsPerYear;
  const assetExcess = mean(assetReturns) - periodRiskFree;
  const benchmarkExcess = mean(benchmarkReturns) - periodRiskFree;
  return (assetExcess - beta(assetReturns, benchmarkReturns) * benchmarkExcess) * periodsPerYear;
}

/**
 * 跟踪误差（年化）。
 *
 * @param assetReturns - 资产收益率
 * @param benchmarkReturns - 基准收益率
 * @param periodsPerYear - 每年期数
 * @returns 主动收益标准差 * sqrt(periodsPerYear)
 */
export function trackingError(
  assetReturns: readonly number[],
  benchmarkReturns: readonly number[],
  periodsPerYear: number,
): number {
  assertPairedSeries(assetReturns, benchmarkReturns);
  assertPositiveFinite(periodsPerYear, 'periodsPerYear');
  const active = assetReturns.map((value, index) => value - (benchmarkReturns[index] ?? 0));
  return sampleStdDev(active) * Math.sqrt(periodsPerYear);
}

/**
 * 信息比率。
 *
 * @param assetReturns - 资产收益率
 * @param benchmarkReturns - 基准收益率
 * @param periodsPerYear - 每年期数
 * @returns 主动收益均值 / 主动收益标准差 * sqrt(periodsPerYear)
 * @throws {MathValueError} 主动收益标准差为 0
 */
export function informationRatio(
  assetReturns: readonly number[],
  benchmarkReturns: readonly number[],
  periodsPerYear: number,
): number {
  assertPairedSeries(assetReturns, benchmarkReturns);
  assertPositiveFinite(periodsPerYear, 'periodsPerYear');
  const active = assetReturns.map((value, index) => value - (benchmarkReturns[index] ?? 0));
  const deviation = sampleStdDev(active);
  if (isNearZero(deviation)) {
    throw new MathValueError('主动收益标准差为 0，无法计算信息比率');
  }
  return (mean(active) / deviation) * Math.sqrt(periodsPerYear);
}

/**
 * 历史 VaR。
 *
 * @param returns - 收益率序列
 * @param confidence - 置信度，`0..1` 开区间，缺省 0.95
 * @returns 正数表示的损失；无损失时返回 0
 */
export function historicalVaR(returns: readonly number[], confidence = 0.95): number {
  assertReturns(returns, false);
  assertConfidence(confidence);
  const sorted = [...returns].sort((left, right) => left - right);
  const rawIndex = (1 - confidence) * sorted.length;
  const index = Math.min(sorted.length - 1, Math.max(0, Math.floor(rawIndex + 1e-9)));
  const quantile = sorted[index] ?? 0;
  return Math.max(0, -quantile);
}

/**
 * 参数 VaR（正态假设）。
 *
 * @param returns - 收益率序列，长度 >= 2
 * @param confidence - 置信度，`0..1` 开区间，缺省 0.95
 * @returns 正数表示的损失
 */
export function parametricVaR(returns: readonly number[], confidence = 0.95): number {
  assertReturns(returns, false);
  assertConfidence(confidence);
  // VaR 关注左尾：使用 1-confidence 对应的分位（负 z），而不是 confidence 对应的上尾分位。
  const z = inverseNormalCdf(1 - confidence);
  const quantile = mean(returns) + z * sampleStdDev(returns);
  return Math.max(0, -quantile);
}

/**
 * 期望损失（CVaR / Expected Shortfall）。
 *
 * @param returns - 收益率序列
 * @param confidence - 置信度，`0..1` 开区间，缺省 0.95
 * @returns 尾部平均损失（正数）
 */
export function expectedShortfall(returns: readonly number[], confidence = 0.95): number {
  assertReturns(returns, false);
  assertConfidence(confidence);
  const sorted = [...returns].sort((left, right) => left - right);
  const rawCount = (1 - confidence) * sorted.length;
  const count = Math.max(1, Math.min(sorted.length, Math.floor(rawCount + 1e-9)));
  const tail = sorted.slice(0, count);
  return Math.max(0, -mean(tail));
}

/**
 * 复合年增长率。
 *
 * @param start - 起始值，> 0
 * @param end - 结束值，>= 0
 * @param years - 年数，> 0
 * @returns `(end/start)^(1/years)-1`
 */
export function compoundAnnualGrowthRate(start: number, end: number, years: number): number {
  assertPositiveFinite(start, 'start');
  assertFinite(end, 'end');
  assertPositiveFinite(years, 'years');
  if (end < 0) {
    throw new MathValueError('end 必须大于等于 0');
  }
  return Math.pow(end / start, 1 / years) - 1;
}

/** 浮点近零判定。 */
function isNearZero(value: number): boolean {
  return Math.abs(value) < ZERO_TOLERANCE;
}

/** 算术平均。 */
function mean(values: readonly number[]): number {
  let total = 0;
  for (const value of values) total += value;
  return total / values.length;
}

/** 样本方差。 */
function sampleVariance(values: readonly number[]): number {
  const average = mean(values);
  let total = 0;
  for (const value of values) {
    const difference = value - average;
    total += difference * difference;
  }
  return total / (values.length - 1);
}

/** 样本标准差。 */
function sampleStdDev(values: readonly number[]): number {
  return Math.sqrt(sampleVariance(values));
}

/** 校验价格序列。 */
function assertPrices(prices: readonly number[]): void {
  if (!Array.isArray(prices) || prices.length < 2) {
    throw new MathValueError('价格序列长度必须 >= 2');
  }
  for (let index = 0; index < prices.length; index += 1) {
    const value = prices[index];
    if (value === undefined || !Number.isFinite(value) || value <= 0) {
      throw new MathValueError(`prices[${index}] 必须是正有限数（实际值: ${String(value)}）`);
    }
  }
}

/** 校验收益率序列。 */
function assertReturns(returns: readonly number[], shouldBeAboveMinusOne: boolean): void {
  if (!Array.isArray(returns) || returns.length < (shouldBeAboveMinusOne ? 1 : 2)) {
    throw new MathValueError(`收益率序列长度必须 >= ${shouldBeAboveMinusOne ? 1 : 2}`);
  }
  for (let index = 0; index < returns.length; index += 1) {
    const value = returns[index];
    if (value === undefined || !Number.isFinite(value)) {
      throw new MathValueError(`returns[${index}] 必须是有限数值（实际值: ${String(value)}）`);
    }
    if (shouldBeAboveMinusOne && value <= -1) {
      throw new MathValueError(`returns[${index}] 必须大于 -1（实际值: ${value}）`);
    }
  }
}

/** 校验成对序列。 */
function assertPairedSeries(left: readonly number[], right: readonly number[]): void {
  assertReturns(left, false);
  assertReturns(right, false);
  if (left.length !== right.length) {
    throw new MathValueError(`序列长度必须相同（${left.length} vs ${right.length}）`);
  }
}

/** 校验置信度。 */
function assertConfidence(confidence: number): void {
  assertFinite(confidence, 'confidence');
  if (confidence <= 0 || confidence >= 1) {
    throw new MathValueError(`confidence 必须在 (0,1) 内（实际值: ${confidence}）`);
  }
}

/** 校验有限数值。 */
function assertFinite(value: number, path: string): void {
  if (!Number.isFinite(value)) {
    throw new MathValueError(`${path} 必须是有限数值（实际值: ${String(value)}）`);
  }
}

/** 校验正有限数值。 */
function assertPositiveFinite(value: number, path: string): void {
  assertFinite(value, path);
  if (value <= 0) {
    throw new MathValueError(`${path} 必须大于 0（实际值: ${value}）`);
  }
}

/** 标准正态分位函数（Acklam 近似，误差约 1e-9）。 */
function inverseNormalCdf(probability: number): number {
  const a1 = -3.969683028665376e1;
  const a2 = 2.209460984245205e2;
  const a3 = -2.759285104469687e2;
  const a4 = 1.38357751867269e2;
  const a5 = -3.066479806614716e1;
  const a6 = 2.506628277459239;
  const b1 = -5.447609879822406e1;
  const b2 = 1.615858368580409e2;
  const b3 = -1.556989798598866e2;
  const b4 = 6.680131188771972e1;
  const b5 = -1.328068155288572e1;
  const c1 = -7.784894002430293e-3;
  const c2 = -3.223964580411365e-1;
  const c3 = -2.400758277161838;
  const c4 = -2.549732539343734;
  const c5 = 4.374664141464968;
  const c6 = 2.938163982698783;
  const d1 = 7.784695709041462e-3;
  const d2 = 3.224671290700398e-1;
  const d3 = 2.445134137142996;
  const d4 = 3.754408661907416;
  const low = 0.02425;
  const high = 1 - low;

  if (probability < low) {
    const q = Math.sqrt(-2 * Math.log(probability));
    return (
      (((((c1 * q + c2) * q + c3) * q + c4) * q + c5) * q + c6) /
      ((((d1 * q + d2) * q + d3) * q + d4) * q + 1)
    );
  }
  if (probability <= high) {
    const q = probability - 0.5;
    const r = q * q;
    return (
      ((((((a1 * r + a2) * r + a3) * r + a4) * r + a5) * r + a6) * q) /
      (((((b1 * r + b2) * r + b3) * r + b4) * r + b5) * r + 1)
    );
  }
  const q = Math.sqrt(-2 * Math.log(1 - probability));
  return (
    -(((((c1 * q + c2) * q + c3) * q + c4) * q + c5) * q + c6) /
    ((((d1 * q + d2) * q + d3) * q + d4) * q + 1)
  );
}
