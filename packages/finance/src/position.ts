/**
 * @file 持仓与盈亏：加权成本、已实现/未实现盈亏、手续费与成交重放。
 * @description 持仓净数量带符号（正多负空）；所有金额使用 quote 货币与 `Decimal` 精确计算。
 * @module finance/position
 */

import { Decimal } from '@kurumi/math';
import { CurrencyMismatchError, FinanceDefinitionError, FinanceValueError } from './errors.js';
import {
  assertSameInstrument,
  instrumentKey,
  isInstrument,
  notionalMoney,
  type Instrument,
} from './instrument.js';
import {
  addMoney,
  createMoney,
  currencyEquals,
  currencyLabel,
  subtractMoney,
  zeroMoney,
  type Money,
} from './money.js';
import { OrderSide, type Fill } from './order.js';
import { assertPositiveDecimal, assertTimestampMs } from './validation.js';

/** 持仓隐含的平均成本计算精度。 */
const POSITION_AVERAGE_SCALE = 18;

/** 持仓。 */
export interface Position {
  /** 标的。 */
  readonly instrument: Instrument;
  /** 带符号净数量：正数多头、负数空头、零为空仓。 */
  readonly netQuantity: Decimal;
  /** 平均开仓价；空仓时为 0。 */
  readonly averageEntryPrice: Decimal;
  /** 毛已实现盈亏（不含手续费），quote 货币。 */
  readonly grossRealizedPnl: Money;
  /** 累计手续费，quote 货币。 */
  readonly fees: Money;
  /** 最后更新时间（epoch 毫秒）。 */
  readonly updatedAtMs: number;
}

/** 应用成交后的持仓更新结果。 */
export interface PositionUpdate {
  /** 新持仓。 */
  readonly position: Position;
  /** 本次成交带来的毛已实现盈亏增量。 */
  readonly realizedPnlDelta: Money;
  /** 本次成交手续费。 */
  readonly fee: Money;
}

/** 盯市结果。 */
export interface PositionMark {
  /** 持仓。 */
  readonly position: Position;
  /** 标记价格。 */
  readonly markPrice: Decimal;
  /** 未实现盈亏。 */
  readonly unrealizedPnl: Money;
  /** 持仓名义价值。 */
  readonly notional: Money;
  /** 毛已实现 + 未实现 - 手续费。 */
  readonly totalPnl: Money;
}

/**
 * 创建空仓。
 *
 * @param instrument - 标的
 * @param timestampMs - 初始时间，缺省 0
 * @returns 冻结空仓
 * @throws {FinanceDefinitionError} 标的非法
 * @throws {FinanceDefinitionError} 时间戳非法
 */
export function createPosition(instrument: Instrument, timestampMs = 0): Position {
  if (!isInstrument(instrument)) {
    throw new FinanceDefinitionError('position.instrument 非法');
  }
  assertTimestampMs(timestampMs, 'position.timestampMs');
  const zero = zeroMoney(instrument.quoteCurrency);
  return Object.freeze({
    instrument,
    netQuantity: Decimal.ZERO,
    averageEntryPrice: Decimal.ZERO,
    grossRealizedPnl: zero,
    fees: zero,
    updatedAtMs: timestampMs,
  });
}

/**
 * 判断是否为空仓。
 *
 * @param position - 持仓
 * @returns 净数量为 0 时 true
 */
export function positionIsFlat(position: Position): boolean {
  return position.netQuantity.isZero();
}

/**
 * 应用成交到持仓。
 *
 * @param position - 当前持仓
 * @param fill - 成交
 * @returns 新持仓、毛已实现盈亏增量与手续费
 * @throws {InstrumentMismatchError} 标的不一致
 * @throws {CurrencyMismatchError} 手续费币种不是 quote
 * @throws {FinanceValueError} 价格/数量/时间非法或时间倒退
 *
 * @example
 * ```ts
 * const first = applyFillToPosition(empty, buyFill);
 * const closed = applyFillToPosition(first.position, sellFill);
 * closed.position.netQuantity.isZero(); // true
 * ```
 */
export function applyFillToPosition(position: Position, fill: Fill): PositionUpdate {
  assertSameInstrument(position.instrument, fill.instrument);
  if (!fill.price.isPositive()) {
    throw new FinanceValueError(`fill.price 必须大于 0（实际值: ${fill.price.toString()}）`);
  }
  if (!fill.quantity.isPositive()) {
    throw new FinanceValueError(`fill.quantity 必须大于 0（实际值: ${fill.quantity.toString()}）`);
  }
  if (!Number.isSafeInteger(fill.timestampMs) || fill.timestampMs < 0) {
    throw new FinanceValueError(
      `fill.timestampMs 必须是非负安全整数（实际值: ${String(fill.timestampMs)}）`,
    );
  }
  if (fill.timestampMs < position.updatedAtMs) {
    throw new FinanceValueError(
      `fill.timestampMs ${fill.timestampMs} 早于持仓更新时间 ${position.updatedAtMs}`,
    );
  }

  const quote = position.instrument.quoteCurrency;
  const fee = fill.fee ?? zeroMoney(quote);
  if (!currencyEquals(fee.currency, quote)) {
    throw new CurrencyMismatchError(currencyLabel(quote), currencyLabel(fee.currency));
  }

  const oldQuantity = position.netQuantity;
  const delta = fill.side === OrderSide.Buy ? fill.quantity : fill.quantity.negate();
  const newQuantity = oldQuantity.add(delta);
  const realizedPnlDelta = zeroMoney(quote);
  let averageEntryPrice = position.averageEntryPrice;

  if (oldQuantity.isZero()) {
    averageEntryPrice = fill.price;
  } else if (oldQuantity.sign() === delta.sign()) {
    averageEntryPrice = oldQuantity
      .abs()
      .multiply(position.averageEntryPrice)
      .add(fill.quantity.multiply(fill.price))
      .divide(oldQuantity.abs().add(fill.quantity), {
        scale: POSITION_AVERAGE_SCALE,
        rounding: 'half-even',
      });
  } else {
    const oldAbsolute = oldQuantity.abs();
    const closedQuantity = oldAbsolute.compare(fill.quantity) <= 0 ? oldAbsolute : fill.quantity;
    const direction = Decimal.fromInteger(oldQuantity.sign());
    const pnlPerUnit = fill.price
      .subtract(position.averageEntryPrice)
      .multiply(direction)
      .multiply(position.instrument.contractMultiplier);
    const realized = createMoney(pnlPerUnit.multiply(closedQuantity), quote);

    if (newQuantity.isZero()) {
      averageEntryPrice = Decimal.ZERO;
    } else if (newQuantity.sign() !== oldQuantity.sign()) {
      averageEntryPrice = fill.price;
    }

    return Object.freeze({
      position: Object.freeze({
        ...position,
        netQuantity: newQuantity,
        averageEntryPrice,
        grossRealizedPnl: addMoney(position.grossRealizedPnl, realized),
        fees: addMoney(position.fees, fee),
        updatedAtMs: fill.timestampMs,
      }),
      realizedPnlDelta: realized,
      fee,
    });
  }

  return Object.freeze({
    position: Object.freeze({
      ...position,
      netQuantity: newQuantity,
      averageEntryPrice,
      fees: addMoney(position.fees, fee),
      updatedAtMs: fill.timestampMs,
    }),
    realizedPnlDelta,
    fee,
  });
}

/**
 * 计算未实现盈亏。
 *
 * @param position - 持仓
 * @param markPrice - 标记价格，必须 > 0
 * @returns `(mark - avg) * netQuantity * multiplier`；空仓返回零
 * @throws {FinanceDefinitionError} markPrice 非法
 */
export function positionUnrealizedPnl(position: Position, markPrice: Decimal): Money {
  assertPositiveDecimal(markPrice, 'markPrice');
  if (position.netQuantity.isZero()) return zeroMoney(position.instrument.quoteCurrency);
  return createMoney(
    markPrice
      .subtract(position.averageEntryPrice)
      .multiply(position.netQuantity)
      .multiply(position.instrument.contractMultiplier),
    position.instrument.quoteCurrency,
  );
}

/**
 * 计算持仓名义价值。
 *
 * @param position - 持仓
 * @param markPrice - 标记价格，必须 > 0
 * @returns `|netQuantity| * markPrice * multiplier`
 */
export function positionNotional(position: Position, markPrice: Decimal): Money {
  return notionalMoney(position.instrument, markPrice, position.netQuantity.abs());
}

/**
 * 计算开仓成本名义价值。
 *
 * @param position - 持仓
 * @returns `|netQuantity| * averageEntryPrice * multiplier`；空仓为零
 */
export function positionEntryNotional(position: Position): Money {
  if (position.netQuantity.isZero()) return zeroMoney(position.instrument.quoteCurrency);
  return createMoney(
    position.averageEntryPrice
      .multiply(position.netQuantity.abs())
      .multiply(position.instrument.contractMultiplier),
    position.instrument.quoteCurrency,
  );
}

/**
 * 计算总盈亏。
 *
 * @param position - 持仓
 * @param markPrice - 标记价格
 * @returns 毛已实现 + 未实现 - 手续费
 */
export function positionTotalPnl(position: Position, markPrice: Decimal): Money {
  return subtractMoney(
    addMoney(position.grossRealizedPnl, positionUnrealizedPnl(position, markPrice)),
    position.fees,
  );
}

/**
 * 盯市快照。
 *
 * @param position - 持仓
 * @param markPrice - 标记价格
 * @returns 冻结的未实现盈亏、名义价值与总盈亏
 */
export function positionMarkToMarket(position: Position, markPrice: Decimal): PositionMark {
  return Object.freeze({
    position,
    markPrice,
    unrealizedPnl: positionUnrealizedPnl(position, markPrice),
    notional: positionNotional(position, markPrice),
    totalPnl: positionTotalPnl(position, markPrice),
  });
}

/**
 * 计算持仓收益率。
 *
 * @param position - 持仓
 * @param markPrice - 标记价格
 * @returns 总盈亏 / 开仓成本；空仓或成本为零时 undefined
 */
export function positionReturn(position: Position, markPrice: Decimal): Decimal | undefined {
  const entryNotional = positionEntryNotional(position);
  if (entryNotional.amount.isZero()) return undefined;
  return positionTotalPnl(position, markPrice).amount.divide(entryNotional.amount, {
    scale: 8,
    rounding: 'half-even',
  });
}

/**
 * 按顺序重放成交，得到持仓。
 *
 * @param instrument - 标的
 * @param fills - 成交序列；必须按时间非降序
 * @param timestampMs - 初始时间，缺省 0
 * @returns 最终持仓
 * @throws {FinanceValueError} 时间倒退或成交非法
 */
export function positionFromFills(
  instrument: Instrument,
  fills: readonly Fill[],
  timestampMs = 0,
): Position {
  let position = createPosition(instrument, timestampMs);
  for (const fill of fills) {
    position = applyFillToPosition(position, fill).position;
  }
  return position;
}

/**
 * 生成持仓的人类可读 key（标的 key）。
 *
 * @param position - 持仓
 * @returns 标的 key
 */
export function positionKey(position: Position): string {
  return instrumentKey(position.instrument);
}
