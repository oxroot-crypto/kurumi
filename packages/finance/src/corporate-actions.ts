/**
 * @file 公司行为：拆股、股票股利、现金分红与历史价格/数量调整。
 * @description 仅提供确定性的价格/数量调整与总回报计算；不处理并购、换股、供股与税务。
 * @module finance/corporate-actions
 */

import { Decimal } from '@kurumi/math';
import { FinanceDefinitionError } from './errors.js';
import { isMoney, type Money } from './money.js';
import {
  assertNonNegativeDecimal,
  assertPositiveDecimal,
  assertTimestampMs,
} from './validation.js';

/** 公司行为类型。 */
export const CorporateActionType = {
  /** 拆股。 */
  Split: 'split',
  /** 股票股利。 */
  StockDividend: 'stock_dividend',
  /** 现金分红。 */
  CashDividend: 'cash_dividend',
} as const;

/** 公司行为类型联合。 */
export type CorporateActionType = (typeof CorporateActionType)[keyof typeof CorporateActionType];

/** 拆股。 */
export interface StockSplit {
  /** 类型。 */
  readonly type: 'split';
  /** 拆股比例；2 表示 1 拆 2。 */
  readonly ratio: Decimal;
  /** 除权日（epoch 毫秒）。 */
  readonly exDateMs: number;
}

/** 股票股利。 */
export interface StockDividend {
  /** 类型。 */
  readonly type: 'stock_dividend';
  /** 每股新增股数比例；0.1 表示 10% 股票股利。 */
  readonly ratio: Decimal;
  /** 除权日（epoch 毫秒）。 */
  readonly exDateMs: number;
}

/** 现金分红。 */
export interface CashDividend {
  /** 类型。 */
  readonly type: 'cash_dividend';
  /** 每股现金分红。 */
  readonly amountPerShare: Money;
  /** 除息日（epoch 毫秒）。 */
  readonly exDateMs: number;
}

/** 公司行为判别联合。 */
export type CorporateAction = StockSplit | StockDividend | CashDividend;

/** 历史价格点。 */
export interface PricePoint {
  /** 时间（epoch 毫秒）。 */
  readonly timestampMs: number;
  /** 价格。 */
  readonly price: Decimal;
}

/** 历史数量点。 */
export interface QuantityPoint {
  /** 时间（epoch 毫秒）。 */
  readonly timestampMs: number;
  /** 数量。 */
  readonly quantity: Decimal;
}

/**
 * 创建拆股行为。
 *
 * @param ratio - 拆股比例，> 0
 * @param exDateMs - 除权日
 * @returns 冻结行为
 */
export function createStockSplit(ratio: Decimal, exDateMs: number): StockSplit {
  assertPositiveDecimal(ratio, 'split.ratio');
  assertTimestampMs(exDateMs, 'split.exDateMs');
  return Object.freeze({ type: CorporateActionType.Split, ratio, exDateMs });
}

/**
 * 创建股票股利行为。
 *
 * @param ratio - 新增股数比例，> 0（0.1 表示 10%）
 * @param exDateMs - 除权日
 * @returns 冻结行为
 */
export function createStockDividend(ratio: Decimal, exDateMs: number): StockDividend {
  assertPositiveDecimal(ratio, 'stockDividend.ratio');
  assertTimestampMs(exDateMs, 'stockDividend.exDateMs');
  return Object.freeze({ type: CorporateActionType.StockDividend, ratio, exDateMs });
}

/**
 * 创建现金分红行为。
 *
 * @param amountPerShare - 每股现金分红，>= 0
 * @param exDateMs - 除息日
 * @returns 冻结行为
 */
export function createCashDividend(amountPerShare: Money, exDateMs: number): CashDividend {
  if (!isMoney(amountPerShare)) {
    throw new FinanceDefinitionError('cashDividend.amountPerShare 非法');
  }
  assertNonNegativeDecimal(amountPerShare.amount, 'cashDividend.amountPerShare');
  assertTimestampMs(exDateMs, 'cashDividend.exDateMs');
  return Object.freeze({
    type: CorporateActionType.CashDividend,
    amountPerShare,
    exDateMs,
  });
}

/**
 * 调整拆股后的价格。
 *
 * @param price - 原价格，> 0
 * @param ratio - 拆股比例，> 0
 * @returns `price / ratio`
 */
export function adjustPriceForSplit(price: Decimal, ratio: Decimal): Decimal {
  assertPositiveDecimal(price, 'price');
  assertPositiveDecimal(ratio, 'ratio');
  return price.divide(ratio, { scale: 18, rounding: 'half-even' });
}

/**
 * 调整拆股后的数量。
 *
 * @param quantity - 原数量，>= 0
 * @param ratio - 拆股比例，> 0
 * @returns `quantity * ratio`
 */
export function adjustQuantityForSplit(quantity: Decimal, ratio: Decimal): Decimal {
  if (quantity.isNegative()) {
    throw new FinanceDefinitionError(`quantity 不能为负（实际值: ${quantity.toString()}）`);
  }
  assertPositiveDecimal(ratio, 'ratio');
  return quantity.multiply(ratio);
}

/**
 * 含现金分红的总回报。
 *
 * @param startPrice - 期初价格，> 0
 * @param endPrice - 期末价格，>= 0
 * @param dividendsPerShare - 期间每股现金分红，>= 0
 * @returns `(end - start + dividends) / start`
 */
export function totalReturnWithDividends(
  startPrice: Decimal,
  endPrice: Decimal,
  dividendsPerShare: Decimal,
): Decimal {
  assertPositiveDecimal(startPrice, 'startPrice');
  if (endPrice.isNegative()) {
    throw new FinanceDefinitionError('endPrice 不能为负');
  }
  assertNonNegativeDecimal(dividendsPerShare, 'dividendsPerShare');
  return endPrice
    .subtract(startPrice)
    .add(dividendsPerShare)
    .divide(startPrice, { scale: 8, rounding: 'half-even' });
}

/**
 * 按拆股/股票股利调整历史价格。
 *
 * @param points - 历史价格点
 * @param actions - 公司行为列表；现金分红不改变价格，仅用于总回报计算
 * @returns 新数组；除权日之后的行为会调整其之前的价格
 * @throws {FinanceDefinitionError} 价格/行为非法
 */
export function adjustHistoricPrices(
  points: readonly PricePoint[],
  actions: readonly CorporateAction[],
): readonly PricePoint[] {
  for (const point of points) {
    assertTimestampMs(point.timestampMs, 'pricePoint.timestampMs');
    assertPositiveDecimal(point.price, 'pricePoint.price');
  }
  for (const action of actions) assertCorporateAction(action);

  const ratioActions = actions.filter(
    (action): action is StockSplit | StockDividend =>
      action.type === CorporateActionType.Split ||
      action.type === CorporateActionType.StockDividend,
  );

  return Object.freeze(
    points.map((point) => {
      let ratio = Decimal.ONE;
      for (const action of ratioActions) {
        if (action.exDateMs > point.timestampMs) {
          ratio = ratio.multiply(actionFactor(action));
        }
      }
      return Object.freeze({
        timestampMs: point.timestampMs,
        price: ratio.equals(Decimal.ONE)
          ? point.price
          : point.price.divide(ratio, { scale: 18, rounding: 'half-even' }),
      });
    }),
  );
}

/**
 * 按拆股/股票股利调整历史数量。
 *
 * @param points - 历史数量点
 * @param actions - 公司行为列表
 * @returns 新数组；除权日之后的行为会调整其之前的数量
 */
export function adjustHistoricQuantities(
  points: readonly QuantityPoint[],
  actions: readonly CorporateAction[],
): readonly QuantityPoint[] {
  for (const point of points) {
    assertTimestampMs(point.timestampMs, 'quantityPoint.timestampMs');
    if (point.quantity.isNegative()) {
      throw new FinanceDefinitionError('quantityPoint.quantity 不能为负');
    }
  }
  for (const action of actions) assertCorporateAction(action);

  const ratioActions = actions.filter(
    (action): action is StockSplit | StockDividend =>
      action.type === CorporateActionType.Split ||
      action.type === CorporateActionType.StockDividend,
  );
  return Object.freeze(
    points.map((point) => {
      let ratio = Decimal.ONE;
      for (const action of ratioActions) {
        if (action.exDateMs > point.timestampMs) {
          ratio = ratio.multiply(actionFactor(action));
        }
      }
      return Object.freeze({
        timestampMs: point.timestampMs,
        quantity: point.quantity.multiply(ratio),
      });
    }),
  );
}

/**
 * 判断值是否为合法公司行为。
 *
 * @param value - 待判断值
 * @returns 结构合法时 true
 */
export function isCorporateAction(value: unknown): value is CorporateAction {
  if (typeof value !== 'object' || value === null) return false;
  const candidate = value as Record<string, unknown>;
  if (
    typeof candidate['exDateMs'] !== 'number' ||
    !Number.isSafeInteger(candidate['exDateMs']) ||
    candidate['exDateMs'] < 0
  ) {
    return false;
  }
  if (
    candidate['type'] === CorporateActionType.Split ||
    candidate['type'] === CorporateActionType.StockDividend
  ) {
    return candidate['ratio'] instanceof Decimal && candidate['ratio'].isPositive();
  }
  if (candidate['type'] === CorporateActionType.CashDividend) {
    return isMoney(candidate['amountPerShare']) && !candidate['amountPerShare'].amount.isNegative();
  }
  return false;
}

/** 拆股为 ratio，股票股利为 1 + ratio。 */
function actionFactor(action: StockSplit | StockDividend): Decimal {
  return action.type === CorporateActionType.StockDividend
    ? Decimal.ONE.add(action.ratio)
    : action.ratio;
}

/** 校验公司行为。 */
function assertCorporateAction(value: unknown): asserts value is CorporateAction {
  if (!isCorporateAction(value)) {
    throw new FinanceDefinitionError('公司行为非法');
  }
}
