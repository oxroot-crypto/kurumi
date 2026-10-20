/**
 * @file 固定收益：债券规格、付息日、应计利息、净价/全价、YTM、久期、凸性与 DV01。
 * @description 债券定价使用 Float64 折现因子；金额输出为 Decimal/Money，仅供估值与风控，不替代清算所结算。
 * @module finance/fixed-income
 */

import { Decimal } from '@kurumi/math';
import { FinanceDefinitionError, FinanceValueError } from './errors.js';
import {
  assertSameCurrency,
  createMoney,
  isCurrency,
  isMoney,
  subtractMoney,
  type Currency,
  type Money,
} from './money.js';
import {
  assertNonEmptyString,
  assertNonNegativeDecimal,
  assertPositiveDecimal,
  assertSafeInteger,
  assertTimestampMs,
} from './validation.js';

/** 每年付息次数。 */
export const CouponFrequency = {
  /** 每年一次。 */
  Annual: 1,
  /** 每半年一次。 */
  SemiAnnual: 2,
  /** 每季度一次。 */
  Quarterly: 4,
  /** 每月一次。 */
  Monthly: 12,
} as const;

/** 付息频率联合类型。 */
export type CouponFrequency = (typeof CouponFrequency)[keyof typeof CouponFrequency];

/** 全部付息频率。 */
const COUPON_FREQUENCIES: readonly CouponFrequency[] = Object.freeze(
  Object.values(CouponFrequency),
);

/** 日计数规则。 */
export const DayCountConvention = {
  /** 实际天数 / 365.25（近似）。 */
  ActualActual: 'actual/actual',
  /** 实际天数 / 360。 */
  Actual360: 'actual/360',
  /** 实际天数 / 365。 */
  Actual365: 'actual/365',
  /** 美国 30/360。 */
  Thirty360: '30/360',
} as const;

/** 日计数规则联合类型。 */
export type DayCountConvention = (typeof DayCountConvention)[keyof typeof DayCountConvention];

/** 全部日计数规则。 */
const DAY_COUNT_CONVENTIONS: readonly DayCountConvention[] = Object.freeze(
  Object.values(DayCountConvention),
);

/** 一天的毫秒数。 */
const MS_PER_DAY = 86_400_000;

/** 债券静态规格。 */
export interface BondSpec {
  /** 债券 id。 */
  readonly id: string;
  /** 面值，> 0。 */
  readonly faceValue: Decimal;
  /** 年化票面利率，>= 0。 */
  readonly couponRate: Decimal;
  /** 每年付息次数。 */
  readonly couponFrequency: CouponFrequency;
  /** 发行日（epoch 毫秒）。 */
  readonly issueDateMs: number;
  /** 到期日（epoch 毫秒）。 */
  readonly maturityDateMs: number;
  /** 日计数规则。 */
  readonly dayCount: DayCountConvention;
  /** 计价货币。 */
  readonly currency: Currency;
}

/** 债券现金流。 */
export interface BondCashFlow {
  /** 支付日（epoch 毫秒）。 */
  readonly dateMs: number;
  /** 票息。 */
  readonly coupon: Money;
  /** 本金。 */
  readonly principal: Money;
  /** 合计。 */
  readonly total: Money;
}

/**
 * 创建债券规格。
 *
 * @param input - 债券 id、面值、票息、频率、日期、日计数与货币
 * @returns 冻结债券规格
 * @throws {FinanceDefinitionError} 字段非法
 */
export function createBondSpec(input: BondSpec): BondSpec {
  assertNonEmptyString(input.id, 'bond.id');
  assertPositiveDecimal(input.faceValue, 'bond.faceValue');
  assertNonNegativeDecimal(input.couponRate, 'bond.couponRate');
  if (!COUPON_FREQUENCIES.includes(input.couponFrequency)) {
    throw new FinanceDefinitionError(
      `bond.couponFrequency 必须是 1/2/4/12（实际值: ${String(input.couponFrequency)}）`,
    );
  }
  assertTimestampMs(input.issueDateMs, 'bond.issueDateMs');
  assertTimestampMs(input.maturityDateMs, 'bond.maturityDateMs');
  if (input.maturityDateMs <= input.issueDateMs) {
    throw new FinanceDefinitionError('bond.maturityDateMs 必须晚于 issueDateMs');
  }
  if (!DAY_COUNT_CONVENTIONS.includes(input.dayCount)) {
    throw new FinanceDefinitionError(`bond.dayCount 非法（实际值: ${String(input.dayCount)}）`);
  }
  if (!isCurrency(input.currency)) {
    throw new FinanceDefinitionError('bond.currency 非法');
  }
  return Object.freeze({
    id: input.id,
    faceValue: input.faceValue,
    couponRate: input.couponRate,
    couponFrequency: input.couponFrequency,
    issueDateMs: input.issueDateMs,
    maturityDateMs: input.maturityDateMs,
    dayCount: input.dayCount,
    currency: input.currency,
  });
}

/**
 * 判断值是否为合法债券规格。
 *
 * @param value - 待判断值
 * @returns 结构合法时 true
 */
export function isBondSpec(value: unknown): value is BondSpec {
  if (typeof value !== 'object' || value === null) return false;
  const candidate = value as Record<string, unknown>;
  return (
    typeof candidate['id'] === 'string' &&
    candidate['id'].trim() !== '' &&
    candidate['faceValue'] instanceof Decimal &&
    candidate['faceValue'].isPositive() &&
    candidate['couponRate'] instanceof Decimal &&
    !candidate['couponRate'].isNegative() &&
    typeof candidate['couponFrequency'] === 'number' &&
    COUPON_FREQUENCIES.includes(candidate['couponFrequency'] as CouponFrequency) &&
    typeof candidate['issueDateMs'] === 'number' &&
    Number.isSafeInteger(candidate['issueDateMs']) &&
    candidate['issueDateMs'] >= 0 &&
    typeof candidate['maturityDateMs'] === 'number' &&
    Number.isSafeInteger(candidate['maturityDateMs']) &&
    candidate['maturityDateMs'] > candidate['issueDateMs'] &&
    typeof candidate['dayCount'] === 'string' &&
    DAY_COUNT_CONVENTIONS.includes(candidate['dayCount'] as DayCountConvention) &&
    isCurrency(candidate['currency'])
  );
}

/**
 * 每个付息周期的票息金额。
 *
 * @param bond - 债券规格
 * @returns quote 货币票息；零息债为 0
 */
export function couponPerPeriod(bond: BondSpec): Money {
  assertBond(bond);
  return createMoney(
    bond.faceValue
      .multiply(bond.couponRate, { scale: 30, rounding: 'half-even' })
      .divide(Decimal.fromInteger(bond.couponFrequency), {
        scale: 18,
        rounding: 'half-even',
      }),
    bond.currency,
  );
}

/**
 * 生成付息日。
 *
 * @param bond - 债券规格
 * @returns 严格递增、位于发行日之后且不晚于到期日的付息日
 */
export function couponSchedule(bond: BondSpec): readonly number[] {
  assertBond(bond);
  const monthsPerPeriod = 12 / bond.couponFrequency;
  const dates: number[] = [];
  for (let period = 1; ; period += 1) {
    const date = addMonthsUtc(bond.issueDateMs, period * monthsPerPeriod);
    if (date >= bond.maturityDateMs) break;
    dates.push(date);
  }
  dates.push(bond.maturityDateMs);
  return Object.freeze(dates);
}

/**
 * 计算两个 epoch 毫秒之间的日历天数（UTC）。
 *
 * @param startMs - 起始时间
 * @param endMs - 结束时间
 * @returns 日历天数，可为负
 */
export function daysBetween(startMs: number, endMs: number): number {
  return utcDayNumber(endMs) - utcDayNumber(startMs);
}

/**
 * 按日计数规则计算年化时间。
 *
 * @param startMs - 起始时间
 * @param endMs - 结束时间
 * @param convention - 日计数规则
 * @returns 年化时间，可为负
 */
export function yearFraction(
  startMs: number,
  endMs: number,
  convention: DayCountConvention,
): number {
  const days = daysBetween(startMs, endMs);
  switch (convention) {
    case DayCountConvention.ActualActual:
      return days / 365.25;
    case DayCountConvention.Actual360:
      return days / 360;
    case DayCountConvention.Actual365:
      return days / 365;
    case DayCountConvention.Thirty360:
      return thirty360Days(startMs, endMs) / 360;
    default: {
      const exhaustive: never = convention;
      throw new FinanceDefinitionError(`未知日计数规则: ${String(exhaustive)}`);
    }
  }
}

/**
 * 应计利息。
 *
 * @param bond - 债券规格
 * @param settlementDateMs - 结算日
 * @returns 应计利息；结算日等于发行日时为 0
 * @throws {FinanceValueError} 结算日越界
 */
export function accruedInterest(bond: BondSpec, settlementDateMs: number): Money {
  assertBond(bond);
  assertSettlement(bond, settlementDateMs);
  if (settlementDateMs <= bond.issueDateMs) {
    return createMoney(Decimal.ZERO, bond.currency);
  }
  const schedule = couponSchedule(bond);
  const previous = findPreviousCouponDate(bond, schedule, settlementDateMs);
  const next = schedule.find((date) => date > settlementDateMs) ?? bond.maturityDateMs;
  const coupon = couponPerPeriod(bond);
  const accruedDays = dayCountDays(previous, settlementDateMs, bond.dayCount);
  const periodDays = dayCountDays(previous, next, bond.dayCount);
  if (periodDays <= 0) {
    throw new FinanceValueError('付息周期天数必须大于 0');
  }
  return createMoney(
    coupon.amount
      .multiply(Decimal.fromInteger(accruedDays))
      .divide(Decimal.fromInteger(periodDays), {
        scale: 18,
        rounding: 'half-even',
      }),
    bond.currency,
  );
}

/**
 * 债券剩余现金流。
 *
 * @param bond - 债券规格
 * @param settlementDateMs - 结算日
 * @returns 冻结现金流列表；最后一期包含本金
 * @throws {FinanceValueError} 结算日越界
 */
export function bondCashFlows(bond: BondSpec, settlementDateMs: number): readonly BondCashFlow[] {
  assertBond(bond);
  assertSettlement(bond, settlementDateMs);
  const coupon = couponPerPeriod(bond);
  const zero = createMoney(Decimal.ZERO, bond.currency);
  const schedule = couponSchedule(bond);
  const cashFlows: BondCashFlow[] = [];
  for (const dateMs of schedule) {
    if (dateMs <= settlementDateMs) continue;
    const isFinal = dateMs === bond.maturityDateMs;
    const principal = isFinal ? createMoney(bond.faceValue, bond.currency) : zero;
    cashFlows.push(
      Object.freeze({
        dateMs,
        coupon,
        principal,
        total: createMoney(coupon.amount.add(principal.amount), bond.currency),
      }),
    );
  }
  if (cashFlows.length === 0) {
    throw new FinanceValueError('债券没有剩余现金流');
  }
  return Object.freeze(cashFlows);
}

/**
 * 债券全价（dirty price）。
 *
 * @param bond - 债券规格
 * @param settlementDateMs - 结算日
 * @param yieldRate - 年化到期收益率
 * @returns 全价
 * @throws {FinanceValueError} 收益率非法或结算日越界
 */
export function bondDirtyPrice(
  bond: BondSpec,
  settlementDateMs: number,
  yieldRate: Decimal,
): Money {
  assertBond(bond);
  assertSettlement(bond, settlementDateMs);
  if (!(yieldRate instanceof Decimal)) {
    throw new FinanceValueError('yieldRate 必须是 Decimal');
  }
  const yieldNumber = yieldRate.toNumber();
  if (1 + yieldNumber <= 0) {
    throw new FinanceValueError(`yieldRate 必须大于 -1（实际值: ${yieldRate.toString()}）`);
  }
  const cashFlows = bondCashFlows(bond, settlementDateMs);
  let price = Decimal.ZERO;
  for (const cashFlow of cashFlows) {
    const time = yearFraction(settlementDateMs, cashFlow.dateMs, bond.dayCount);
    const factor = 1 / Math.pow(1 + yieldNumber, time);
    price = price.add(
      cashFlow.total.amount.multiply(decimalFromNumber(factor, 'bond discount factor'), {
        scale: 18,
        rounding: 'half-even',
      }),
    );
  }
  return createMoney(price, bond.currency);
}

/**
 * 债券净价（clean price）。
 *
 * @param bond - 债券规格
 * @param settlementDateMs - 结算日
 * @param yieldRate - 年化到期收益率
 * @returns 全价 - 应计利息
 */
export function bondCleanPrice(
  bond: BondSpec,
  settlementDateMs: number,
  yieldRate: Decimal,
): Money {
  return subtractMoney(
    bondDirtyPrice(bond, settlementDateMs, yieldRate),
    accruedInterest(bond, settlementDateMs),
  );
}

/**
 * 由净价反解到期收益率。
 *
 * @param bond - 债券规格
 * @param settlementDateMs - 结算日
 * @param cleanPrice - 目标净价
 * @returns 年化 YTM
 * @throws {FinanceValueError} 价格越界或未收敛
 */
export function bondYield(bond: BondSpec, settlementDateMs: number, cleanPrice: Money): Decimal {
  assertBond(bond);
  assertSettlement(bond, settlementDateMs);
  if (!isMoney(cleanPrice)) {
    throw new FinanceValueError('cleanPrice 必须是 Money');
  }
  assertSameCurrency(cleanPrice, createMoney(Decimal.ZERO, bond.currency));
  if (cleanPrice.amount.isNegative() || cleanPrice.amount.isZero()) {
    throw new FinanceValueError('cleanPrice 必须大于 0');
  }
  let low = Decimal.parse('-0.99');
  let high = Decimal.parse('10');
  const lowPrice = bondCleanPrice(bond, settlementDateMs, low);
  const highPrice = bondCleanPrice(bond, settlementDateMs, high);
  if (
    cleanPrice.amount.compare(lowPrice.amount) > 0 ||
    cleanPrice.amount.compare(highPrice.amount) < 0
  ) {
    throw new FinanceValueError(
      `目标价格 ${cleanPrice.amount.toString()} 超出可求解区间 [${highPrice.amount.toString()}, ${lowPrice.amount.toString()}]`,
    );
  }
  const tolerance = Decimal.parse('0.000000001');
  for (let iteration = 0; iteration < 100; iteration += 1) {
    const mid = low.add(high).divide(Decimal.fromInteger(2), {
      scale: 18,
      rounding: 'half-even',
    });
    const price = bondCleanPrice(bond, settlementDateMs, mid);
    if (price.amount.subtract(cleanPrice.amount).abs().compare(tolerance) <= 0) {
      return mid;
    }
    if (price.amount.compare(cleanPrice.amount) > 0) {
      low = mid;
    } else {
      high = mid;
    }
    if (high.subtract(low).abs().compare(Decimal.parse('0.000000000001')) <= 0) {
      return mid;
    }
  }
  throw new FinanceValueError('YTM 在最大迭代次数内未收敛');
}

/**
 * 当前收益率。
 *
 * @param bond - 债券规格
 * @param cleanPrice - 净价，必须 > 0
 * @returns 年化票息 / 净价
 */
export function currentYield(bond: BondSpec, cleanPrice: Money): Decimal {
  assertBond(bond);
  if (!isMoney(cleanPrice)) {
    throw new FinanceValueError('cleanPrice 必须是 Money');
  }
  assertSameCurrency(cleanPrice, createMoney(Decimal.ZERO, bond.currency));
  if (!cleanPrice.amount.isPositive()) {
    throw new FinanceValueError('cleanPrice 必须大于 0');
  }
  return bond.faceValue
    .multiply(bond.couponRate, { scale: 30, rounding: 'half-even' })
    .divide(cleanPrice.amount, { scale: 8, rounding: 'half-even' });
}

/**
 * Macaulay 久期。
 *
 * @param bond - 债券规格
 * @param settlementDateMs - 结算日
 * @param yieldRate - 年化 YTM
 * @returns 久期（年）
 */
export function macaulayDuration(
  bond: BondSpec,
  settlementDateMs: number,
  yieldRate: Decimal,
): Decimal {
  const dirtyPrice = bondDirtyPrice(bond, settlementDateMs, yieldRate);
  if (!dirtyPrice.amount.isPositive()) {
    throw new FinanceValueError('dirty price 必须大于 0 才能计算久期');
  }
  const yieldNumber = yieldRate.toNumber();
  let weighted = Decimal.ZERO;
  for (const cashFlow of bondCashFlows(bond, settlementDateMs)) {
    const time = yearFraction(settlementDateMs, cashFlow.dateMs, bond.dayCount);
    const factor = 1 / Math.pow(1 + yieldNumber, time);
    const presentValue = cashFlow.total.amount.multiply(
      decimalFromNumber(factor, 'duration factor'),
      { scale: 18, rounding: 'half-even' },
    );
    weighted = weighted.add(
      presentValue.multiply(decimalFromNumber(time, 'duration time'), {
        scale: 18,
        rounding: 'half-even',
      }),
    );
  }
  return weighted.divide(dirtyPrice.amount, { scale: 8, rounding: 'half-even' });
}

/**
 * 修正久期。
 *
 * @param bond - 债券规格
 * @param settlementDateMs - 结算日
 * @param yieldRate - 年化 YTM
 * @returns `Macaulay / (1+y)`
 */
export function modifiedDuration(
  bond: BondSpec,
  settlementDateMs: number,
  yieldRate: Decimal,
): Decimal {
  return macaulayDuration(bond, settlementDateMs, yieldRate).divide(Decimal.ONE.add(yieldRate), {
    scale: 8,
    rounding: 'half-even',
  });
}

/**
 * 凸性。
 *
 * @param bond - 债券规格
 * @param settlementDateMs - 结算日
 * @param yieldRate - 年化 YTM
 * @returns 凸性
 */
export function convexity(bond: BondSpec, settlementDateMs: number, yieldRate: Decimal): Decimal {
  const dirtyPrice = bondDirtyPrice(bond, settlementDateMs, yieldRate);
  if (!dirtyPrice.amount.isPositive()) {
    throw new FinanceValueError('dirty price 必须大于 0 才能计算凸性');
  }
  const yieldNumber = yieldRate.toNumber();
  let weighted = Decimal.ZERO;
  for (const cashFlow of bondCashFlows(bond, settlementDateMs)) {
    const time = yearFraction(settlementDateMs, cashFlow.dateMs, bond.dayCount);
    const factor = 1 / Math.pow(1 + yieldNumber, time);
    const pv = cashFlow.total.amount.multiply(decimalFromNumber(factor, 'convexity factor'), {
      scale: 18,
      rounding: 'half-even',
    });
    weighted = weighted.add(
      pv.multiply(decimalFromNumber(time * (time + 1), 'convexity time'), {
        scale: 18,
        rounding: 'half-even',
      }),
    );
  }
  const onePlusYield = Decimal.ONE.add(yieldRate);
  const denominator = dirtyPrice.amount
    .multiply(onePlusYield, { scale: 18, rounding: 'half-even' })
    .multiply(onePlusYield, { scale: 18, rounding: 'half-even' });
  return weighted.divide(denominator, { scale: 8, rounding: 'half-even' });
}

/**
 * DV01（1bp 价格变动）。
 *
 * @param bond - 债券规格
 * @param settlementDateMs - 结算日
 * @param yieldRate - 年化 YTM
 * @returns 修正久期 * 全价 * 0.0001
 */
export function dv01(bond: BondSpec, settlementDateMs: number, yieldRate: Decimal): Money {
  const duration = modifiedDuration(bond, settlementDateMs, yieldRate);
  const dirtyPrice = bondDirtyPrice(bond, settlementDateMs, yieldRate);
  return createMoney(
    dirtyPrice.amount.multiply(duration).multiply(Decimal.parse('0.0001')),
    bond.currency,
  );
}

/** 校验债券规格。 */
function assertBond(bond: BondSpec): void {
  if (!isBondSpec(bond)) throw new FinanceDefinitionError('bond 规格非法');
}

/** 校验结算日。 */
function assertSettlement(bond: BondSpec, settlementDateMs: number): void {
  assertSafeInteger(settlementDateMs, 'settlementDateMs');
  if (settlementDateMs < bond.issueDateMs || settlementDateMs >= bond.maturityDateMs) {
    throw new FinanceValueError(
      `settlementDateMs 必须位于 [issueDateMs, maturityDateMs) 内（实际值: ${settlementDateMs}）`,
    );
  }
}

/** 查找上一个付息日或发行日。 */
function findPreviousCouponDate(
  bond: BondSpec,
  schedule: readonly number[],
  settlementDateMs: number,
): number {
  let previous = bond.issueDateMs;
  for (const date of schedule) {
    if (date > settlementDateMs) break;
    previous = date;
  }
  return previous;
}

/** 按日计数规则返回天数。 */
function dayCountDays(startMs: number, endMs: number, convention: DayCountConvention): number {
  return convention === DayCountConvention.Thirty360
    ? thirty360Days(startMs, endMs)
    : daysBetween(startMs, endMs);
}

/** UTC 日期序号。 */
function utcDayNumber(timestampMs: number): number {
  const date = new Date(timestampMs);
  return Math.floor(
    Date.UTC(date.getUTCFullYear(), date.getUTCMonth(), date.getUTCDate()) / MS_PER_DAY,
  );
}

/** 增加月份（UTC，月末夹紧）。 */
function addMonthsUtc(timestampMs: number, months: number): number {
  const date = new Date(timestampMs);
  const year = date.getUTCFullYear();
  const month = date.getUTCMonth() + months;
  const target = new Date(Date.UTC(year, month, 1));
  const lastDay = new Date(
    Date.UTC(target.getUTCFullYear(), target.getUTCMonth() + 1, 0),
  ).getUTCDate();
  target.setUTCDate(Math.min(date.getUTCDate(), lastDay));
  return target.getTime();
}

/** 美国 30/360 天数。 */
function thirty360Days(startMs: number, endMs: number): number {
  const start = new Date(startMs);
  const end = new Date(endMs);
  let day1 = start.getUTCDate();
  let day2 = end.getUTCDate();
  if (day1 === 31) day1 = 30;
  if (day2 === 31 && day1 === 30) day2 = 30;
  return (
    360 * (end.getUTCFullYear() - start.getUTCFullYear()) +
    30 * (end.getUTCMonth() - start.getUTCMonth()) +
    (day2 - day1)
  );
}

/** 把有限 Float64 转为 Decimal（最多 12 位小数）。 */
function decimalFromNumber(value: number, label: string): Decimal {
  if (!Number.isFinite(value)) {
    throw new FinanceValueError(`${label} 不是有限数（实际值: ${String(value)}）`);
  }
  return Decimal.parse(value.toFixed(12));
}
