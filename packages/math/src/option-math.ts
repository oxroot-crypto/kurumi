/**
 * @file 期权定价数学：内在价值、货币性、盈亏平衡、Black-Scholes、Greeks 与隐含波动率。
 * @description 与具体标的/货币解耦；使用 Float64 超越函数，输出为模型数值，不直接进入账务。
 * @module math/option-math
 */

import { Decimal } from './decimal.js';
import { MathValueError, OptionModelError } from './errors.js';

/** 期权方向。 */
export const OptionKind = {
  /** 看涨。 */
  Call: 'call',
  /** 看跌。 */
  Put: 'put',
} as const;

/** 期权方向联合。 */
export type OptionKind = (typeof OptionKind)[keyof typeof OptionKind];

/** 全部期权方向。 */
const OPTION_KINDS: readonly OptionKind[] = Object.freeze(Object.values(OptionKind));

/** Black-Scholes 输入。 */
export interface BlackScholesInput {
  /** 标的现价，> 0。 */
  readonly spot: number;
  /** 行权价，> 0。 */
  readonly strike: number;
  /** 到期时间（年），>= 0。 */
  readonly timeToExpiryYears: number;
  /** 年化波动率（0.2 表示 20%），>= 0。 */
  readonly volatility: number;
  /** 年化无风险利率（0.05 表示 5%）。 */
  readonly riskFreeRate: number;
  /** 年化分红率，缺省 0。 */
  readonly dividendYield?: number;
  /** call / put。 */
  readonly optionType: OptionKind;
}

/** Black-Scholes 结果。 */
export interface BlackScholesResult {
  /** 每单位标的的期权价格。 */
  readonly price: number;
  /** Delta。 */
  readonly delta: number;
  /** Gamma。 */
  readonly gamma: number;
  /** Vega：对波动率 1.0 的敏感度。 */
  readonly vega: number;
  /** Theta：对 1 年的敏感度。 */
  readonly theta: number;
  /** Rho：对利率 1.0 的敏感度。 */
  readonly rho: number;
}

/** 隐含波动率输入（不含 volatility）。 */
export type ImpliedVolatilityInput = Omit<BlackScholesInput, 'volatility'>;

/** 隐含波动率选项。 */
export interface ImpliedVolatilityOptions {
  /** 最大波动率，缺省 5。 */
  readonly maxVolatility?: number;
  /** 二分容差，缺省 1e-8。 */
  readonly tolerance?: number;
  /** 最大迭代次数，缺省 100。 */
  readonly maxIterations?: number;
}

/**
 * 计算内在价值（每单位标的）。
 *
 * @param optionType - call / put
 * @param underlyingPrice - 标的现价，> 0
 * @param strike - 行权价，> 0
 * @returns call 为 `max(S-K,0)`，put 为 `max(K-S,0)`
 */
export function optionIntrinsicValue(
  optionType: OptionKind,
  underlyingPrice: Decimal,
  strike: Decimal,
): Decimal {
  assertOptionKind(optionType);
  assertPositiveDecimalValue(underlyingPrice, 'underlyingPrice');
  assertPositiveDecimalValue(strike, 'strike');
  const difference =
    optionType === OptionKind.Call
      ? underlyingPrice.subtract(strike)
      : strike.subtract(underlyingPrice);
  return difference.isPositive() ? difference : Decimal.ZERO;
}

/**
 * 计算货币性。
 *
 * @param underlyingPrice - 标的现价，> 0
 * @param strike - 行权价，> 0
 * @returns `underlyingPrice / strike`，scale 8、half-even
 */
export function optionMoneyness(underlyingPrice: Decimal, strike: Decimal): Decimal {
  assertPositiveDecimalValue(underlyingPrice, 'underlyingPrice');
  assertPositiveDecimalValue(strike, 'strike');
  return underlyingPrice.divide(strike, { scale: 8, rounding: 'half-even' });
}

/**
 * 判断是否实值。
 *
 * @param optionType - call / put
 * @param underlyingPrice - 标的现价
 * @param strike - 行权价
 * @returns call 在 S > K、put 在 S < K 时 true
 */
export function optionIsInTheMoney(
  optionType: OptionKind,
  underlyingPrice: Decimal,
  strike: Decimal,
): boolean {
  assertOptionKind(optionType);
  assertPositiveDecimalValue(underlyingPrice, 'underlyingPrice');
  assertPositiveDecimalValue(strike, 'strike');
  return optionType === OptionKind.Call
    ? underlyingPrice.compare(strike) > 0
    : underlyingPrice.compare(strike) < 0;
}

/**
 * 计算盈亏平衡点。
 *
 * @param optionType - call / put
 * @param strike - 行权价
 * @param premium - 每单位标的期权费，>= 0
 * @returns call 为 `strike + premium`，put 为 `strike - premium`
 */
export function optionBreakeven(
  optionType: OptionKind,
  strike: Decimal,
  premium: Decimal,
): Decimal {
  assertOptionKind(optionType);
  assertPositiveDecimalValue(strike, 'strike');
  if (!(premium instanceof Decimal) || premium.isNegative()) {
    throw new MathValueError('premium 必须是非负 Decimal');
  }
  return optionType === OptionKind.Call ? strike.add(premium) : strike.subtract(premium);
}

/**
 * Black-Scholes 定价与 Greeks。
 *
 * @param input - 现价、行权价、期限、波动率、利率、分红率与方向
 * @returns 价格与 Greeks
 * @throws {OptionModelError} 参数非法
 */
export function blackScholes(input: BlackScholesInput): BlackScholesResult {
  const { spot, strike, timeToExpiryYears, volatility, riskFreeRate } = input;
  const dividendYield = input.dividendYield ?? 0;

  assertModelNumber(spot, 'spot', true);
  assertModelNumber(strike, 'strike', true);
  assertModelNumber(timeToExpiryYears, 'timeToExpiryYears', false);
  assertModelNumber(volatility, 'volatility', false);
  assertModelNumber(riskFreeRate, 'riskFreeRate', false, true);
  assertModelNumber(dividendYield, 'dividendYield', false, true);
  if (input.optionType !== OptionKind.Call && input.optionType !== OptionKind.Put) {
    throw new OptionModelError(`optionType 必须是 call/put（实际值: ${String(input.optionType)}）`);
  }

  if (timeToExpiryYears === 0) {
    return atExpiryResult(spot, strike, input.optionType);
  }
  if (volatility === 0) {
    return zeroVolatilityResult(
      spot,
      strike,
      timeToExpiryYears,
      riskFreeRate,
      dividendYield,
      input.optionType,
    );
  }

  const sqrtT = Math.sqrt(timeToExpiryYears);
  const d1 =
    (Math.log(spot / strike) +
      (riskFreeRate - dividendYield + 0.5 * volatility * volatility) * timeToExpiryYears) /
    (volatility * sqrtT);
  const d2 = d1 - volatility * sqrtT;
  const discountSpot = spot * Math.exp(-dividendYield * timeToExpiryYears);
  const discountStrike = strike * Math.exp(-riskFreeRate * timeToExpiryYears);
  const nd1 = normalCdf(d1);
  const nd2 = normalCdf(d2);
  const pdf = normalPdf(d1);

  const isCall = input.optionType === OptionKind.Call;
  const price = isCall
    ? discountSpot * nd1 - discountStrike * nd2
    : discountStrike * normalCdf(-d2) - discountSpot * normalCdf(-d1);
  const delta = isCall
    ? Math.exp(-dividendYield * timeToExpiryYears) * nd1
    : -Math.exp(-dividendYield * timeToExpiryYears) * normalCdf(-d1);
  const gamma = (Math.exp(-dividendYield * timeToExpiryYears) * pdf) / (spot * volatility * sqrtT);
  const vega = discountSpot * pdf * sqrtT;
  const theta = isCall
    ? (-discountSpot * pdf * volatility) / (2 * sqrtT) -
      riskFreeRate * discountStrike * nd2 +
      dividendYield * discountSpot * nd1
    : (-discountSpot * pdf * volatility) / (2 * sqrtT) +
      riskFreeRate * discountStrike * normalCdf(-d2) -
      dividendYield * discountSpot * normalCdf(-d1);
  const rho = isCall
    ? strike * timeToExpiryYears * Math.exp(-riskFreeRate * timeToExpiryYears) * nd2
    : -strike * timeToExpiryYears * Math.exp(-riskFreeRate * timeToExpiryYears) * normalCdf(-d2);

  return Object.freeze({
    price: Math.max(0, price),
    delta,
    gamma,
    vega,
    theta,
    rho,
  });
}

/**
 * Black-Scholes 价格。
 *
 * @param input - 定价输入
 * @returns 每单位标的期权价格
 */
export function blackScholesPrice(input: BlackScholesInput): number {
  return blackScholes(input).price;
}

/**
 * 由市场价格反解隐含波动率。
 *
 * @param input - 不含 volatility 的定价输入
 * @param marketPrice - 市场价格
 * @param options - 最大波动率/容差/迭代次数
 * @returns 隐含波动率
 * @throws {OptionModelError} 价格越界、参数非法或未收敛
 */
export function impliedVolatility(
  input: ImpliedVolatilityInput,
  marketPrice: number,
  options?: ImpliedVolatilityOptions,
): number {
  assertModelNumber(input.spot, 'spot', true);
  assertModelNumber(input.strike, 'strike', true);
  assertModelNumber(input.timeToExpiryYears, 'timeToExpiryYears', false);
  assertModelNumber(input.riskFreeRate, 'riskFreeRate', false, true);
  assertModelNumber(input.dividendYield ?? 0, 'dividendYield', false, true);
  if (!Number.isFinite(marketPrice) || marketPrice < 0) {
    throw new OptionModelError(`marketPrice 必须是非负有限数（实际值: ${String(marketPrice)}）`);
  }
  if (input.optionType !== OptionKind.Call && input.optionType !== OptionKind.Put) {
    throw new OptionModelError(`optionType 必须是 call/put（实际值: ${String(input.optionType)}）`);
  }

  const time = input.timeToExpiryYears;
  const dividendYield = input.dividendYield ?? 0;
  const discountSpot = input.spot * Math.exp(-dividendYield * time);
  const discountStrike = input.strike * Math.exp(-input.riskFreeRate * time);
  const isCall = input.optionType === OptionKind.Call;
  const lowerBound = Math.max(
    isCall ? discountSpot - discountStrike : discountStrike - discountSpot,
    0,
  );
  const upperBound = isCall ? discountSpot : discountStrike;
  const tolerance = options?.tolerance ?? 1e-8;
  if (!Number.isFinite(tolerance) || tolerance <= 0) {
    throw new OptionModelError(`tolerance 必须是正有限数（实际值: ${String(tolerance)}）`);
  }
  if (marketPrice < lowerBound - tolerance || marketPrice > upperBound + tolerance) {
    throw new OptionModelError(
      `marketPrice ${marketPrice} 超出无套利区间 [${lowerBound}, ${upperBound}]`,
    );
  }
  if (time === 0 || marketPrice <= lowerBound + tolerance) {
    return 0;
  }

  const maxVolatility = options?.maxVolatility ?? 5;
  const maxIterations = options?.maxIterations ?? 100;
  if (!Number.isFinite(maxVolatility) || maxVolatility <= 0) {
    throw new OptionModelError(`maxVolatility 必须是正有限数（实际值: ${String(maxVolatility)}）`);
  }
  if (!Number.isSafeInteger(maxIterations) || maxIterations <= 0) {
    throw new OptionModelError(
      `maxIterations 必须是正安全整数（实际值: ${String(maxIterations)}）`,
    );
  }

  let low = 0;
  let high = maxVolatility;
  const highPrice = blackScholes({ ...input, volatility: high }).price;
  if (marketPrice > highPrice + tolerance) {
    throw new OptionModelError(
      `marketPrice ${marketPrice} 超过 maxVolatility=${maxVolatility} 对应的价格 ${highPrice}`,
    );
  }

  for (let iteration = 0; iteration < maxIterations; iteration += 1) {
    const mid = (low + high) / 2;
    const price = blackScholes({ ...input, volatility: mid }).price;
    if (Math.abs(price - marketPrice) <= tolerance) return mid;
    if (price < marketPrice) {
      low = mid;
    } else {
      high = mid;
    }
    if (high - low <= tolerance) return (low + high) / 2;
  }

  throw new OptionModelError(
    `隐含波动率在 ${maxIterations} 次迭代内未收敛（区间 [${low}, ${high}]）`,
  );
}

/** 到期结果。 */
function atExpiryResult(spot: number, strike: number, optionType: OptionKind): BlackScholesResult {
  const isCall = optionType === OptionKind.Call;
  const intrinsic = isCall ? Math.max(spot - strike, 0) : Math.max(strike - spot, 0);
  const delta = isCall
    ? spot > strike
      ? 1
      : spot < strike
        ? 0
        : 0.5
    : spot < strike
      ? -1
      : spot > strike
        ? 0
        : -0.5;
  return Object.freeze({
    price: intrinsic,
    delta,
    gamma: 0,
    vega: 0,
    theta: 0,
    rho: 0,
  });
}

/** 零波动率结果。 */
function zeroVolatilityResult(
  spot: number,
  strike: number,
  timeToExpiryYears: number,
  riskFreeRate: number,
  dividendYield: number,
  optionType: OptionKind,
): BlackScholesResult {
  const discountSpot = spot * Math.exp(-dividendYield * timeToExpiryYears);
  const discountStrike = strike * Math.exp(-riskFreeRate * timeToExpiryYears);
  const isCall = optionType === OptionKind.Call;
  const price = isCall
    ? Math.max(discountSpot - discountStrike, 0)
    : Math.max(discountStrike - discountSpot, 0);
  const delta = isCall
    ? discountSpot > discountStrike
      ? Math.exp(-dividendYield * timeToExpiryYears)
      : 0
    : discountSpot < discountStrike
      ? -Math.exp(-dividendYield * timeToExpiryYears)
      : 0;
  return Object.freeze({
    price,
    delta,
    gamma: 0,
    vega: 0,
    theta: 0,
    rho: 0,
  });
}

/** 校验期权方向。 */
function assertOptionKind(optionType: OptionKind): void {
  if (!OPTION_KINDS.includes(optionType)) {
    throw new MathValueError(`optionType 必须是 call/put（实际值: ${String(optionType)}）`);
  }
}

/** 校验正 Decimal。 */
function assertPositiveDecimalValue(value: Decimal, path: string): void {
  if (!(value instanceof Decimal) || !value.isPositive()) {
    throw new MathValueError(`${path} 必须是正 Decimal（实际值: ${String(value)}）`);
  }
}

/** 校验模型数值。 */
function assertModelNumber(
  value: number,
  path: string,
  isPositive: boolean,
  allowNegative = false,
): void {
  if (!Number.isFinite(value)) {
    throw new OptionModelError(`${path} 必须是有限数值（实际值: ${String(value)}）`);
  }
  if (isPositive && value <= 0) {
    throw new OptionModelError(`${path} 必须大于 0（实际值: ${value}）`);
  }
  if (!isPositive && !allowNegative && value < 0) {
    throw new OptionModelError(`${path} 必须大于等于 0（实际值: ${value}）`);
  }
}

/** 标准正态分布概率密度。 */
function normalPdf(value: number): number {
  return Math.exp(-0.5 * value * value) / Math.sqrt(2 * Math.PI);
}

/** 标准正态分布累积分布函数。 */
function normalCdf(value: number): number {
  return 0.5 * erfc(-value / Math.SQRT2);
}

/** 互补误差函数（Numerical Recipes erfcc 近似）。 */
function erfc(value: number): number {
  const z = Math.abs(value);
  const t = 1 / (1 + 0.5 * z);
  const result =
    t *
    Math.exp(
      -z * z -
        1.26551223 +
        t *
          (1.00002368 +
            t *
              (0.37409196 +
                t *
                  (0.09678418 +
                    t *
                      (-0.18628806 +
                        t *
                          (0.27886807 +
                            t *
                              (-1.13520398 +
                                t * (1.48851587 + t * (-0.82215223 + t * 0.17087277)))))))),
    );
  return value >= 0 ? result : 2 - result;
}
