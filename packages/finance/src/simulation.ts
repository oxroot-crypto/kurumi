/**
 * @file 基于报价的撮合模拟：市价/限价/止损触发、盘口数量与 FOK 语义。
 * @description 这是纯函数撮合助手，不维护订单簿队列、不处理手续费；真实执行由上层自行实现，finance 不提供执行类 provider。
 * @module finance/simulation
 */

import { Decimal, minDecimal } from '@kurumi/math';
import { FinanceValueError } from './errors.js';
import { instrumentKey, sameInstrument } from './instrument.js';
import type { Quote } from './market-data.js';
import { OrderSide, OrderType, TimeInForce, isOrderOpen, type Order } from './order.js';

/** 撮合选项。 */
export interface MatchOrderOptions {
  /** 最新价；stop/stop-limit 订单必填。 */
  readonly lastPrice?: Decimal;
  /** 本次最多成交数量；缺省使用订单剩余数量。 */
  readonly maxQuantity?: Decimal;
}

/** 撮合结果。 */
export interface OrderMatch {
  /** 成交数量。 */
  readonly fillQuantity: Decimal;
  /** 成交价格。 */
  readonly fillPrice: Decimal;
}

/**
 * 判断止损单是否触发。
 *
 * @param order - stop 或 stop-limit 订单
 * @param lastPrice - 最新成交价
 * @returns buy stop 在 `last >= stop`、sell stop 在 `last <= stop` 时触发
 * @throws {FinanceValueError} 订单不是止损单或缺少 stopPrice
 */
export function isStopTriggered(order: Order, lastPrice: Decimal): boolean {
  if (order.type !== OrderType.Stop && order.type !== OrderType.StopLimit) {
    throw new FinanceValueError(`订单 ${order.id} 不是止损单，无法判断触发`);
  }
  const stopPrice = order.stopPrice;
  if (stopPrice === undefined) {
    throw new FinanceValueError(`订单 ${order.id} 缺少 stopPrice`);
  }
  return order.side === OrderSide.Buy
    ? lastPrice.compare(stopPrice) >= 0
    : lastPrice.compare(stopPrice) <= 0;
}

/**
 * 用当前报价撮合订单。
 *
 * @param order - open/partially_filled 订单
 * @param quote - 与订单标的相同的报价
 * @param options - 最新价（止损触发）与最大成交数量
 * @returns 撮合结果；未触发/不可成交/无盘口数量时 undefined
 * @throws {FinanceValueError} 订单状态/标的/参数非法
 *
 * @example
 * ```ts
 * const match = matchOrderAgainstQuote(order, quote);
 * if (match !== undefined) {
 *   const fill = createFill({
 *     id: 'f-1', orderId: order.id, instrument: order.instrument,
 *     side: order.side, price: match.fillPrice, quantity: match.fillQuantity,
 *     timestampMs: quote.timestampMs,
 *   });
 *   const next = applyFill(order, fill);
 * }
 * ```
 */
export function matchOrderAgainstQuote(
  order: Order,
  quote: Quote,
  options?: MatchOrderOptions,
): OrderMatch | undefined {
  if (!isOrderOpen(order)) {
    throw new FinanceValueError(
      `订单 ${order.id} 状态 ${order.status} 不可撮合（仅 open/partially_filled）`,
    );
  }
  if (!sameInstrument(order.instrument, quote.instrument)) {
    throw new FinanceValueError(
      `报价标的不一致（订单 ${instrumentKey(order.instrument)}，报价 ${instrumentKey(quote.instrument)}）`,
    );
  }

  if (order.type === OrderType.Stop || order.type === OrderType.StopLimit) {
    const lastPrice = options?.lastPrice;
    if (lastPrice === undefined) {
      throw new FinanceValueError(`止损订单 ${order.id} 撮合必须提供 lastPrice`);
    }
    if (!isStopTriggered(order, lastPrice)) return undefined;
  }

  const isBuy = order.side === OrderSide.Buy;
  const bookPrice = isBuy ? quote.ask : quote.bid;
  const bookSize = isBuy ? quote.askSize : quote.bidSize;

  if (
    (order.type === OrderType.Limit || order.type === OrderType.StopLimit) &&
    order.limitPrice !== undefined
  ) {
    const isMarketable = isBuy
      ? bookPrice.compare(order.limitPrice) <= 0
      : bookPrice.compare(order.limitPrice) >= 0;
    if (!isMarketable) return undefined;
  }

  if (!bookSize.isPositive()) return undefined;

  const maxQuantity = options?.maxQuantity ?? order.remainingQuantity;
  if (!maxQuantity.isPositive()) {
    throw new FinanceValueError(`maxQuantity 必须大于 0（实际值: ${maxQuantity.toString()}）`);
  }

  const fillQuantity = minDecimal([order.remainingQuantity, bookSize, maxQuantity]);
  if (order.timeInForce === TimeInForce.Fok && fillQuantity.compare(order.remainingQuantity) < 0) {
    return undefined;
  }

  return Object.freeze({ fillQuantity, fillPrice: bookPrice });
}
