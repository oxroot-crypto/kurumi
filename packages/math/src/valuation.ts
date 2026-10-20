/**
 * @file 估值模型：CAPM、WACC、DCF、Gordon 增长、NPV/IRR、每股价值与安全边际。
 * @description 使用 Float64 进行模型计算；输出为模型数值，不得直接作为账务金额。
 * @module math/valuation
 */

import { MathValueError } from './errors.js';

/** CAPM 输入。 */
export interface CapmInput {
  /** 无风险利率（0.03 表示 3%）。 */
  readonly riskFreeRate: number;
  /** 贝塔系数。 */
  readonly beta: number;
  /** 市场预期收益率。 */
  readonly marketReturn: number;
}

/** WACC 输入。 */
export interface WaccInput {
  /** 权益市值，>= 0。 */
  readonly equityValue: number;
  /** 债务市值，>= 0。 */
  readonly debtValue: number;
  /** 权益成本。 */
  readonly costOfEquity: number;
  /** 债务成本。 */
  readonly costOfDebt: number;
  /** 税率，`0..1`。 */
  readonly taxRate: number;
}

/** DCF 输入。 */
export interface DiscountedCashFlowInput {
  /** 预测期现金流；第一个元素为第 1 期。 */
  readonly cashFlows: readonly number[];
  /** 折现率（0.1 表示 10%）。 */
  readonly discountRate: number;
  /** 永续增长率；与 terminalValue 互斥。 */
  readonly terminalGrowthRate?: number;
  /** 第 n 期终值；与 terminalGrowthRate 互斥。 */
  readonly terminalValue?: number;
}

/** DCF 结果。 */
export interface DiscountedCashFlowResult {
  /** 预测期现金流现值。 */
  readonly presentValue: number;
  /** 终值现值。 */
  readonly terminalValuePresentValue: number;
  /** 企业价值。 */
  readonly enterpriseValue: number;
}

/** Gordon 增长模型输入。 */
export interface GordonGrowthInput {
  /** 下一期每股股利（D1），>= 0。 */
  readonly dividendPerShare: number;
  /** 要求回报率。 */
  readonly requiredReturn: number;
  /** 永续增长率。 */
  readonly growthRate: number;
}

/**
 * CAPM 权益成本。
 *
 * @param input - 无风险利率、beta 与市场收益率
 * @returns `rf + beta * (market - rf)`
 * @throws {MathValueError} 输入非有限
 */
export function capitalAssetPricingModel(input: CapmInput): number {
  assertFinite(input.riskFreeRate, 'riskFreeRate');
  assertFinite(input.beta, 'beta');
  assertFinite(input.marketReturn, 'marketReturn');
  return input.riskFreeRate + input.beta * (input.marketReturn - input.riskFreeRate);
}

/**
 * 加权平均资本成本。
 *
 * @param input - 权益/债务市值、权益/债务成本与税率
 * @returns `E/V*Ke + D/V*Kd*(1-tax)`
 * @throws {MathValueError} 权重非法或税率越界
 */
export function weightedAverageCostOfCapital(input: WaccInput): number {
  assertNonNegative(input.equityValue, 'equityValue');
  assertNonNegative(input.debtValue, 'debtValue');
  assertFinite(input.costOfEquity, 'costOfEquity');
  assertFinite(input.costOfDebt, 'costOfDebt');
  assertFinite(input.taxRate, 'taxRate');
  if (input.taxRate < 0 || input.taxRate > 1) {
    throw new MathValueError(`taxRate 必须在 [0,1] 内（实际值: ${input.taxRate}）`);
  }
  const total = input.equityValue + input.debtValue;
  if (total <= 0) {
    throw new MathValueError('equityValue 与 debtValue 不能同时为 0');
  }
  return (
    (input.equityValue / total) * input.costOfEquity +
    (input.debtValue / total) * input.costOfDebt * (1 - input.taxRate)
  );
}

/**
 * 现金流折现。
 *
 * @param input - 预测期现金流、折现率与终值参数
 * @returns 预测期现值、终值现值与企业价值
 * @throws {MathValueError} 现金流为空、参数非法或终值参数冲突
 *
 * @example
 * ```ts
 * discountedCashFlow({ cashFlows: [100, 110, 121], discountRate: 0.1,
 *   terminalGrowthRate: 0.02 });
 * ```
 */
export function discountedCashFlow(input: DiscountedCashFlowInput): DiscountedCashFlowResult {
  assertCashFlows(input.cashFlows, 'cashFlows');
  assertFinite(input.discountRate, 'discountRate');
  if (input.discountRate <= -1) {
    throw new MathValueError('discountRate 必须大于 -1');
  }
  if (input.terminalGrowthRate !== undefined && input.terminalValue !== undefined) {
    throw new MathValueError('terminalGrowthRate 与 terminalValue 不能同时提供');
  }

  let presentValue = 0;
  for (let index = 0; index < input.cashFlows.length; index += 1) {
    const cashFlow = input.cashFlows[index];
    if (cashFlow === undefined) continue;
    presentValue += cashFlow / Math.pow(1 + input.discountRate, index + 1);
  }

  let terminalValuePresentValue = 0;
  if (input.terminalValue !== undefined) {
    assertFinite(input.terminalValue, 'terminalValue');
    terminalValuePresentValue =
      input.terminalValue / Math.pow(1 + input.discountRate, input.cashFlows.length);
  } else if (input.terminalGrowthRate !== undefined) {
    assertFinite(input.terminalGrowthRate, 'terminalGrowthRate');
    if (input.discountRate <= input.terminalGrowthRate) {
      throw new MathValueError('discountRate 必须大于 terminalGrowthRate');
    }
    const lastCashFlow = input.cashFlows[input.cashFlows.length - 1];
    if (lastCashFlow === undefined) {
      throw new MathValueError('cashFlows 不能为空');
    }
    const terminalValue =
      (lastCashFlow * (1 + input.terminalGrowthRate)) /
      (input.discountRate - input.terminalGrowthRate);
    terminalValuePresentValue =
      terminalValue / Math.pow(1 + input.discountRate, input.cashFlows.length);
  }

  return Object.freeze({
    presentValue,
    terminalValuePresentValue,
    enterpriseValue: presentValue + terminalValuePresentValue,
  });
}

/**
 * Gordon 永续增长模型。
 *
 * @param input - 下一期股利、要求回报率与增长率
 * @returns `D1/(r-g)`
 * @throws {MathValueError} 参数非法或 `requiredReturn <= growthRate`
 */
export function gordonGrowthModel(input: GordonGrowthInput): number {
  assertNonNegative(input.dividendPerShare, 'dividendPerShare');
  assertFinite(input.requiredReturn, 'requiredReturn');
  assertFinite(input.growthRate, 'growthRate');
  if (input.requiredReturn <= input.growthRate) {
    throw new MathValueError('requiredReturn 必须大于 growthRate');
  }
  return input.dividendPerShare / (input.requiredReturn - input.growthRate);
}

/**
 * 净现值。
 *
 * @param cashFlows - 现金流；第一个元素为第 0 期
 * @param discountRate - 折现率
 * @returns `Σ CF_t/(1+r)^t`
 * @throws {MathValueError} 现金流非法
 */
export function netPresentValue(cashFlows: readonly number[], discountRate: number): number {
  assertCashFlows(cashFlows, 'cashFlows');
  assertFinite(discountRate, 'discountRate');
  if (discountRate <= -1) {
    throw new MathValueError('discountRate 必须大于 -1');
  }
  let total = 0;
  for (let index = 0; index < cashFlows.length; index += 1) {
    const cashFlow = cashFlows[index];
    if (cashFlow === undefined) continue;
    total += cashFlow / Math.pow(1 + discountRate, index);
  }
  return total;
}

/**
 * 内部收益率（有界二分）。
 *
 * @param cashFlows - 现金流；第一个元素为第 0 期，必须至少包含一正一负
 * @param tolerance - 收敛容差，缺省 1e-10
 * @returns IRR
 * @throws {MathValueError} 无符号变化、参数非法或未收敛
 */
export function internalRateOfReturn(cashFlows: readonly number[], tolerance = 1e-10): number {
  assertCashFlows(cashFlows, 'cashFlows');
  assertFinite(tolerance, 'tolerance');
  if (tolerance <= 0) {
    throw new MathValueError('tolerance 必须大于 0');
  }
  const hasPositive = cashFlows.some((value) => value > 0);
  const hasNegative = cashFlows.some((value) => value < 0);
  if (!hasPositive || !hasNegative) {
    throw new MathValueError('IRR 至少需要一正一负现金流');
  }

  let low = -0.99;
  let high = 10;
  let lowValue = netPresentValue(cashFlows, low);
  const highValue = netPresentValue(cashFlows, high);
  if (lowValue === 0) return low;
  if (highValue === 0) return high;
  if (lowValue * highValue > 0) {
    throw new MathValueError('IRR 在 [-0.99, 10] 内无符号变化，无法求解');
  }

  for (let iteration = 0; iteration < 200; iteration += 1) {
    const mid = (low + high) / 2;
    const midValue = netPresentValue(cashFlows, mid);
    if (Math.abs(midValue) <= tolerance || (high - low) / 2 <= tolerance) return mid;
    if (midValue * lowValue < 0) {
      high = mid;
    } else {
      low = mid;
      lowValue = midValue;
    }
  }
  throw new MathValueError('IRR 在最大迭代次数内未收敛');
}

/**
 * 企业价值转权益价值。
 *
 * @param enterpriseValue - 企业价值
 * @param debtValue - 债务价值，>= 0
 * @param cashValue - 现金及等价物，>= 0
 * @returns `EV - debt + cash`
 */
export function equityValueFromEnterpriseValue(
  enterpriseValue: number,
  debtValue: number,
  cashValue: number,
): number {
  assertFinite(enterpriseValue, 'enterpriseValue');
  assertNonNegative(debtValue, 'debtValue');
  assertNonNegative(cashValue, 'cashValue');
  return enterpriseValue - debtValue + cashValue;
}

/**
 * 每股价值。
 *
 * @param equityValue - 权益价值
 * @param shares - 股本，> 0
 * @returns `equityValue / shares`
 * @throws {MathValueError} 参数非法
 */
export function perShareValue(equityValue: number, shares: number): number {
  assertFinite(equityValue, 'equityValue');
  assertFinite(shares, 'shares');
  if (shares <= 0) {
    throw new MathValueError('shares 必须大于 0');
  }
  return equityValue / shares;
}

/**
 * 安全边际。
 *
 * @param intrinsicValue - 内在价值，> 0
 * @param marketPrice - 市场价格，>= 0
 * @returns `(intrinsic - price)/intrinsic`
 * @throws {MathValueError} 参数非法
 */
export function marginOfSafety(intrinsicValue: number, marketPrice: number): number {
  assertFinite(intrinsicValue, 'intrinsicValue');
  assertNonNegative(marketPrice, 'marketPrice');
  if (intrinsicValue <= 0) {
    throw new MathValueError('intrinsicValue 必须大于 0');
  }
  return (intrinsicValue - marketPrice) / intrinsicValue;
}

/**
 * 折现因子。
 *
 * @param rate - 折现率，> -1
 * @param periods - 期数，>= 0；可为小数
 * @returns `1/(1+rate)^periods`
 */
export function discountFactor(rate: number, periods: number): number {
  assertFinite(rate, 'rate');
  assertFinite(periods, 'periods');
  if (rate <= -1) {
    throw new MathValueError('rate 必须大于 -1');
  }
  if (periods < 0) {
    throw new MathValueError('periods 必须大于等于 0');
  }
  return 1 / Math.pow(1 + rate, periods);
}

/** 校验有限数值。 */
function assertFinite(value: number, path: string): void {
  if (!Number.isFinite(value)) {
    throw new MathValueError(`${path} 必须是有限数值（实际值: ${String(value)}）`);
  }
}

/** 校验非负数值。 */
function assertNonNegative(value: number, path: string): void {
  assertFinite(value, path);
  if (value < 0) {
    throw new MathValueError(`${path} 必须大于等于 0（实际值: ${value}）`);
  }
}

/** 校验非空有限现金流。 */
function assertCashFlows(cashFlows: readonly number[], path: string): void {
  if (!Array.isArray(cashFlows) || cashFlows.length === 0) {
    throw new MathValueError(`${path} 不能为空`);
  }
  for (let index = 0; index < cashFlows.length; index += 1) {
    const value = cashFlows[index];
    if (value === undefined || !Number.isFinite(value)) {
      throw new MathValueError(`${path}[${index}] 必须是有限数值（实际值: ${String(value)}）`);
    }
  }
}
