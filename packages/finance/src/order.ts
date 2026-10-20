/**
 * @file 订单与成交：订单构造、状态机、成交应用与派生指标。
 * @description `applyOrderEvent` 是唯一状态转移入口；终态不可再转移，超量/限价/时序违规均显式失败。
 * @module finance/order
 */

import { Decimal, type DecimalRounding } from '@kurumi/math';
import {
  CurrencyMismatchError,
  FinanceDefinitionError,
  OrderOverfillError,
  OrderStateError,
} from './errors.js';
import {
  assertQuantityValid,
  instrumentKey,
  isInstrument,
  isPriceValid,
  notionalMoney,
  sameInstrument,
  type Instrument,
} from './instrument.js';
import { createMoney, currencyEquals, currencyLabel, isMoney, type Money } from './money.js';
import { assertNonEmptyString, assertOneOf, assertTimestampMs } from './validation.js';

/** 订单方向。 */
export const OrderSide = {
  /** 买入/做多。 */
  Buy: 'buy',
  /** 卖出/做空。 */
  Sell: 'sell',
} as const;

/** 订单方向联合类型。 */
export type OrderSide = (typeof OrderSide)[keyof typeof OrderSide];

/** 订单方向集合。 */
const ORDER_SIDES: readonly OrderSide[] = Object.freeze(Object.values(OrderSide));

/** 订单类型。 */
export const OrderType = {
  /** 市价单。 */
  Market: 'market',
  /** 限价单。 */
  Limit: 'limit',
  /** 止损单：触发后转市价。 */
  Stop: 'stop',
  /** 止损限价单：触发后转限价。 */
  StopLimit: 'stop-limit',
} as const;

/** 订单类型联合类型。 */
export type OrderType = (typeof OrderType)[keyof typeof OrderType];

/** 订单类型集合。 */
const ORDER_TYPES: readonly OrderType[] = Object.freeze(Object.values(OrderType));

/** 有效期类型。 */
export const TimeInForce = {
  /** 撤单前一直有效。 */
  Gtc: 'gtc',
  /** 立即成交，剩余撤销。 */
  Ioc: 'ioc',
  /** 全部成交或全部撤销。 */
  Fok: 'fok',
  /** 当日有效。 */
  Day: 'day',
} as const;

/** 有效期联合类型。 */
export type TimeInForce = (typeof TimeInForce)[keyof typeof TimeInForce];

/** 有效期集合。 */
const TIME_IN_FORCE_VALUES: readonly TimeInForce[] = Object.freeze(Object.values(TimeInForce));

/** 订单状态。 */
export const OrderStatus = {
  /** 新建，尚未被接受。 */
  PendingNew: 'pending_new',
  /** 已接受，可成交。 */
  Open: 'open',
  /** 部分成交。 */
  PartiallyFilled: 'partially_filled',
  /** 全部成交。 */
  Filled: 'filled',
  /** 已撤销。 */
  Canceled: 'canceled',
  /** 已拒绝。 */
  Rejected: 'rejected',
  /** 已过期。 */
  Expired: 'expired',
} as const;

/** 订单状态联合类型。 */
export type OrderStatus = (typeof OrderStatus)[keyof typeof OrderStatus];

/** 订单状态集合。 */
const ORDER_STATUSES: readonly OrderStatus[] = Object.freeze(Object.values(OrderStatus));

/** 终态集合。 */
const TERMINAL_STATUSES: ReadonlySet<OrderStatus> = new Set([
  OrderStatus.Filled,
  OrderStatus.Canceled,
  OrderStatus.Rejected,
  OrderStatus.Expired,
]);

/** 可撤销/可过期状态。 */
const ACTIVE_STATUSES: ReadonlySet<OrderStatus> = new Set([
  OrderStatus.PendingNew,
  OrderStatus.Open,
  OrderStatus.PartiallyFilled,
]);

/** 可成交状态。 */
const FILLABLE_STATUSES: ReadonlySet<OrderStatus> = new Set([
  OrderStatus.Open,
  OrderStatus.PartiallyFilled,
]);

/** 流动性角色。 */
export const Liquidity = {
  /** 挂单方。 */
  Maker: 'maker',
  /** 吃单方。 */
  Taker: 'taker',
  /** 未知。 */
  Unknown: 'unknown',
} as const;

/** 流动性联合类型。 */
export type Liquidity = (typeof Liquidity)[keyof typeof Liquidity];

/** 流动性集合。 */
const LIQUIDITY_VALUES: readonly Liquidity[] = Object.freeze(Object.values(Liquidity));

/** 订单 id 规范。 */
const ORDER_ID_PATTERN = /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/;

/** 订单。 */
export interface Order {
  /** 交易所/系统订单 id。 */
  readonly id: string;
  /** 客户端订单 id。 */
  readonly clientOrderId?: string;
  /** 标的。 */
  readonly instrument: Instrument;
  /** 方向。 */
  readonly side: OrderSide;
  /** 类型。 */
  readonly type: OrderType;
  /** 有效期。 */
  readonly timeInForce: TimeInForce;
  /** 下单数量。 */
  readonly quantity: Decimal;
  /** 已成交数量。 */
  readonly filledQuantity: Decimal;
  /** 剩余数量，恒等于 quantity - filledQuantity。 */
  readonly remainingQuantity: Decimal;
  /** 累计成交金额（price * quantity，不含 multiplier）。 */
  readonly filledNotional: Decimal;
  /** 限价；limit/stop-limit 必填。 */
  readonly limitPrice?: Decimal;
  /** 止损触发价；stop/stop-limit 必填。 */
  readonly stopPrice?: Decimal;
  /** 状态。 */
  readonly status: OrderStatus;
  /** 创建时间（epoch 毫秒）。 */
  readonly createdAtMs: number;
  /** 最后更新时间（epoch 毫秒）。 */
  readonly updatedAtMs: number;
  /** 拒绝原因；仅 rejected 状态存在。 */
  readonly rejectReason?: string;
}

/** 订单创建输入。 */
export interface CreateOrderInput {
  /** 订单 id。 */
  readonly id: string;
  /** 客户端订单 id。 */
  readonly clientOrderId?: string;
  /** 标的。 */
  readonly instrument: Instrument;
  /** 方向。 */
  readonly side: OrderSide;
  /** 类型。 */
  readonly type: OrderType;
  /** 有效期，缺省 gtc。 */
  readonly timeInForce?: TimeInForce;
  /** 数量。 */
  readonly quantity: Decimal;
  /** 限价。 */
  readonly limitPrice?: Decimal;
  /** 止损触发价。 */
  readonly stopPrice?: Decimal;
  /** 创建时间（epoch 毫秒）。 */
  readonly createdAtMs: number;
}

/** 成交。 */
export interface Fill {
  /** 成交 id。 */
  readonly id: string;
  /** 订单 id。 */
  readonly orderId: string;
  /** 标的。 */
  readonly instrument: Instrument;
  /** 方向。 */
  readonly side: OrderSide;
  /** 成交价。 */
  readonly price: Decimal;
  /** 成交量。 */
  readonly quantity: Decimal;
  /** 成交时间（epoch 毫秒）。 */
  readonly timestampMs: number;
  /** 手续费；币种必须等于标的 quote。 */
  readonly fee?: Money;
  /** 流动性角色。 */
  readonly liquidity?: Liquidity;
  /** 交易所成交 id。 */
  readonly tradeId?: string;
}

/** 成交创建输入。 */
export interface CreateFillInput {
  /** 成交 id。 */
  readonly id: string;
  /** 订单 id。 */
  readonly orderId: string;
  /** 标的。 */
  readonly instrument: Instrument;
  /** 方向。 */
  readonly side: OrderSide;
  /** 成交价。 */
  readonly price: Decimal;
  /** 成交量。 */
  readonly quantity: Decimal;
  /** 成交时间（epoch 毫秒）。 */
  readonly timestampMs: number;
  /** 手续费。 */
  readonly fee?: Money;
  /** 流动性角色。 */
  readonly liquidity?: Liquidity;
  /** 交易所成交 id。 */
  readonly tradeId?: string;
}

/** 订单状态机事件。 */
export type OrderEvent =
  | { readonly type: 'accept'; readonly timestampMs?: number }
  | { readonly type: 'reject'; readonly reason: string; readonly timestampMs?: number }
  | { readonly type: 'cancel'; readonly timestampMs?: number }
  | { readonly type: 'expire'; readonly timestampMs?: number }
  | { readonly type: 'fill'; readonly fill: Fill };

/**
 * 创建订单（初始状态 pending_new）。
 *
 * @param input - 订单字段
 * @returns 冻结订单
 * @throws {FinanceDefinitionError} id/数量/价格/timestamp/类型矩阵非法
 * @throws {CurrencyMismatchError} 手续费币种与 quote 不一致
 *
 * @example
 * ```ts
 * const order = createOrder({
 *   id: 'o-1',
 *   instrument: btc,
 *   side: 'buy',
 *   type: 'limit',
 *   quantity: Decimal.parse('0.01'),
 *   limitPrice: Decimal.parse('60000'),
 *   createdAtMs: 1,
 * });
 * ```
 */
export function createOrder(input: CreateOrderInput): Order {
  assertOrderId(input.id, 'order.id');
  if (input.clientOrderId !== undefined) {
    assertOrderId(input.clientOrderId, 'order.clientOrderId');
  }
  if (!isInstrument(input.instrument)) {
    throw new FinanceDefinitionError('order.instrument 非法');
  }
  if (!input.instrument.isTradable) {
    throw new FinanceDefinitionError(`标的不允许交易: ${instrumentKey(input.instrument)}`);
  }
  assertOneOf(input.side, ORDER_SIDES, 'order.side');
  assertOneOf(input.type, ORDER_TYPES, 'order.type');
  const timeInForce = input.timeInForce ?? TimeInForce.Gtc;
  assertOneOf(timeInForce, TIME_IN_FORCE_VALUES, 'order.timeInForce');
  assertQuantityValid(input.instrument, input.quantity, 'order.quantity');
  assertTimestampMs(input.createdAtMs, 'order.createdAtMs');

  const limitPrice = input.limitPrice;
  const stopPrice = input.stopPrice;
  assertOrderPriceMatrix(input.type, limitPrice, stopPrice, input.instrument);

  return Object.freeze({
    id: input.id,
    ...(input.clientOrderId === undefined ? {} : { clientOrderId: input.clientOrderId }),
    instrument: input.instrument,
    side: input.side,
    type: input.type,
    timeInForce,
    quantity: input.quantity,
    filledQuantity: Decimal.ZERO,
    remainingQuantity: input.quantity,
    filledNotional: Decimal.ZERO,
    ...(limitPrice === undefined ? {} : { limitPrice }),
    ...(stopPrice === undefined ? {} : { stopPrice }),
    status: OrderStatus.PendingNew,
    createdAtMs: input.createdAtMs,
    updatedAtMs: input.createdAtMs,
  });
}

/**
 * 创建成交。
 *
 * @param input - 成交字段
 * @returns 冻结成交
 * @throws {FinanceDefinitionError} 字段非法
 * @throws {CurrencyMismatchError} 手续费币种与 quote 不一致
 */
export function createFill(input: CreateFillInput): Fill {
  assertOrderId(input.id, 'fill.id');
  assertOrderId(input.orderId, 'fill.orderId');
  if (!isInstrument(input.instrument)) {
    throw new FinanceDefinitionError('fill.instrument 非法');
  }
  assertOneOf(input.side, ORDER_SIDES, 'fill.side');
  assertPositiveFillPrice(input.instrument, input.price, 'fill.price');
  assertPositiveFillQuantity(input.quantity, 'fill.quantity');
  assertTimestampMs(input.timestampMs, 'fill.timestampMs');
  if (input.fee !== undefined) {
    if (!isMoney(input.fee)) {
      throw new FinanceDefinitionError('fill.fee 非法');
    }
    if (!currencyEquals(input.fee.currency, input.instrument.quoteCurrency)) {
      throw new CurrencyMismatchError(
        currencyLabel(input.instrument.quoteCurrency),
        currencyLabel(input.fee.currency),
      );
    }
  }
  const liquidity = input.liquidity ?? Liquidity.Unknown;
  assertOneOf(liquidity, LIQUIDITY_VALUES, 'fill.liquidity');
  if (input.tradeId !== undefined) {
    assertNonEmptyString(input.tradeId, 'fill.tradeId');
  }

  return Object.freeze({
    id: input.id,
    orderId: input.orderId,
    instrument: input.instrument,
    side: input.side,
    price: input.price,
    quantity: input.quantity,
    timestampMs: input.timestampMs,
    ...(input.fee === undefined ? {} : { fee: input.fee }),
    liquidity,
    ...(input.tradeId === undefined ? {} : { tradeId: input.tradeId }),
  });
}

/**
 * 应用订单状态机事件。
 *
 * @param order - 当前订单
 * @param event - 状态机事件
 * @returns 冻结的新订单
 * @throws {OrderStateError} 非法转移、时间倒退、标的方向不一致、限价违规
 * @throws {OrderOverfillError} 成交数量超过剩余
 *
 * @example
 * ```ts
 * const open = applyOrderEvent(order, { type: 'accept', timestampMs: 2 });
 * const filled = applyOrderEvent(open, { type: 'fill', fill });
 * ```
 */
export function applyOrderEvent(order: Order, event: OrderEvent): Order {
  switch (event.type) {
    case 'accept': {
      ensureStatus(order, OrderStatus.PendingNew, 'accept');
      const timestampMs = resolveEventTimestamp(order, event.timestampMs);
      return Object.freeze({
        ...order,
        status: OrderStatus.Open,
        updatedAtMs: timestampMs,
      });
    }
    case 'reject': {
      ensureStatus(order, OrderStatus.PendingNew, 'reject');
      assertNonEmptyString(event.reason, 'reject.reason');
      const timestampMs = resolveEventTimestamp(order, event.timestampMs);
      return Object.freeze({
        ...order,
        status: OrderStatus.Rejected,
        rejectReason: event.reason,
        updatedAtMs: timestampMs,
      });
    }
    case 'cancel': {
      ensureActive(order, 'cancel');
      const timestampMs = resolveEventTimestamp(order, event.timestampMs);
      return Object.freeze({
        ...order,
        status: OrderStatus.Canceled,
        updatedAtMs: timestampMs,
      });
    }
    case 'expire': {
      ensureActive(order, 'expire');
      const timestampMs = resolveEventTimestamp(order, event.timestampMs);
      return Object.freeze({
        ...order,
        status: OrderStatus.Expired,
        updatedAtMs: timestampMs,
      });
    }
    case 'fill': {
      return applyFillEvent(order, event.fill);
    }
    default: {
      const unknown = event as { readonly type?: unknown };
      throw new OrderStateError(order.id, order.status, String(unknown.type), '未知事件');
    }
  }
}

/**
 * 接受订单：pending_new -> open。
 *
 * @param order - 订单
 * @param timestampMs - 事件时间；缺省保持 updatedAt
 * @returns 新订单
 */
export function acceptOrder(order: Order, timestampMs?: number): Order {
  return applyOrderEvent(
    order,
    timestampMs === undefined ? { type: 'accept' } : { type: 'accept', timestampMs },
  );
}

/**
 * 拒绝订单：pending_new -> rejected。
 *
 * @param order - 订单
 * @param reason - 非空拒绝原因
 * @param timestampMs - 事件时间
 * @returns 新订单
 */
export function rejectOrder(order: Order, reason: string, timestampMs?: number): Order {
  return applyOrderEvent(
    order,
    timestampMs === undefined
      ? { type: 'reject', reason }
      : { type: 'reject', reason, timestampMs },
  );
}

/**
 * 撤销订单：pending_new/open/partially_filled -> canceled。
 *
 * @param order - 订单
 * @param timestampMs - 事件时间
 * @returns 新订单
 */
export function cancelOrder(order: Order, timestampMs?: number): Order {
  return applyOrderEvent(
    order,
    timestampMs === undefined ? { type: 'cancel' } : { type: 'cancel', timestampMs },
  );
}

/**
 * 过期订单：pending_new/open/partially_filled -> expired。
 *
 * @param order - 订单
 * @param timestampMs - 事件时间
 * @returns 新订单
 */
export function expireOrder(order: Order, timestampMs?: number): Order {
  return applyOrderEvent(
    order,
    timestampMs === undefined ? { type: 'expire' } : { type: 'expire', timestampMs },
  );
}

/**
 * 应用成交。
 *
 * @param order - 订单
 * @param fill - 成交
 * @returns 新订单
 * @throws {OrderStateError} 状态/标的方向/时间/限价非法
 * @throws {OrderOverfillError} 超量成交
 */
export function applyFill(order: Order, fill: Fill): Order {
  return applyOrderEvent(order, { type: 'fill', fill });
}

/**
 * 判断订单是否处于终态。
 *
 * @param order - 订单
 * @returns filled/canceled/rejected/expired 时 true
 */
export function isOrderTerminal(order: Order): boolean {
  return TERMINAL_STATUSES.has(order.status);
}

/**
 * 判断订单是否可继续成交。
 *
 * @param order - 订单
 * @returns open/partially_filled 时 true
 */
export function isOrderOpen(order: Order): boolean {
  return FILLABLE_STATUSES.has(order.status);
}

/**
 * 计算平均成交价。
 *
 * @param order - 订单
 * @returns `filledNotional / filledQuantity`（scale 18、half-even）；无成交时 undefined
 */
export function orderAverageFillPrice(
  order: Order,
  rounding: DecimalRounding = 'half-even',
): Decimal | undefined {
  if (order.filledQuantity.isZero()) return undefined;
  return order.filledNotional.divide(order.filledQuantity, { scale: 18, rounding });
}

/**
 * 计算累计成交金额（含合约乘数）。
 *
 * @param order - 订单
 * @returns quote 金额
 */
export function orderFilledNotional(order: Order): Money {
  return createMoney(
    order.filledNotional.multiply(order.instrument.contractMultiplier),
    order.instrument.quoteCurrency,
  );
}

/**
 * 按给定价格估算剩余数量名义价值。
 *
 * @param order - 订单
 * @param price - 参考价格，必须 > 0
 * @returns quote 金额
 */
export function orderRemainingNotional(order: Order, price: Decimal): Money {
  return notionalMoney(order.instrument, price, order.remainingQuantity);
}

/**
 * 订单成交进度。
 *
 * @param order - 订单
 * @returns `filledQuantity / quantity`，scale 8、half-even
 */
export function orderProgress(order: Order): Decimal {
  return order.filledQuantity.divide(order.quantity, { scale: 8, rounding: 'half-even' });
}

/** 应用成交事件。 */
function applyFillEvent(order: Order, fill: Fill): Order {
  if (!FILLABLE_STATUSES.has(order.status)) {
    throw new OrderStateError(order.id, order.status, 'fill');
  }
  if (fill.orderId !== order.id) {
    throw new OrderStateError(
      order.id,
      order.status,
      'fill',
      `fill.orderId ${fill.orderId} 与订单不一致`,
    );
  }
  if (!sameInstrument(order.instrument, fill.instrument)) {
    throw new OrderStateError(
      order.id,
      order.status,
      'fill',
      `fill.instrument ${instrumentKey(fill.instrument)} 与订单不一致`,
    );
  }
  if (fill.side !== order.side) {
    throw new OrderStateError(
      order.id,
      order.status,
      'fill',
      `fill.side ${fill.side} 与订单方向 ${order.side} 不一致`,
    );
  }
  if (!(fill.quantity instanceof Decimal) || !fill.quantity.isPositive()) {
    throw new OrderStateError(order.id, order.status, 'fill', 'fill.quantity 必须是正 Decimal');
  }
  if (!(fill.price instanceof Decimal)) {
    throw new OrderStateError(order.id, order.status, 'fill', 'fill.price 必须是 Decimal');
  }
  if (!isPriceValid(order.instrument, fill.price)) {
    throw new OrderStateError(
      order.id,
      order.status,
      'fill',
      `fill.price ${fill.price.toString()} 不是合法 tick`,
    );
  }
  if (!Number.isSafeInteger(fill.timestampMs) || fill.timestampMs < 0) {
    throw new OrderStateError(
      order.id,
      order.status,
      'fill',
      `fill.timestampMs 必须是非负安全整数（实际值: ${String(fill.timestampMs)}）`,
    );
  }
  if (fill.timestampMs < order.updatedAtMs) {
    throw new OrderStateError(
      order.id,
      order.status,
      'fill',
      `fill.timestampMs ${fill.timestampMs} 早于订单更新时间 ${order.updatedAtMs}`,
    );
  }
  if (fill.fee !== undefined) {
    if (!isMoney(fill.fee)) {
      throw new OrderStateError(order.id, order.status, 'fill', 'fill.fee 非法');
    }
    if (!currencyEquals(fill.fee.currency, order.instrument.quoteCurrency)) {
      throw new CurrencyMismatchError(
        currencyLabel(order.instrument.quoteCurrency),
        currencyLabel(fill.fee.currency),
      );
    }
  }
  if (fill.quantity.compare(order.remainingQuantity) > 0) {
    throw new OrderOverfillError(
      order.id,
      order.remainingQuantity.toString(),
      fill.quantity.toString(),
    );
  }
  assertFillPriceWithinLimit(order, fill.price);

  const filledQuantity = order.filledQuantity.add(fill.quantity);
  const remainingQuantity = order.quantity.subtract(filledQuantity);
  const filledNotional = order.filledNotional.add(fill.price.multiply(fill.quantity));
  const status = remainingQuantity.isZero() ? OrderStatus.Filled : OrderStatus.PartiallyFilled;

  return Object.freeze({
    ...order,
    filledQuantity,
    remainingQuantity,
    filledNotional,
    status,
    updatedAtMs: fill.timestampMs,
  });
}

/** 校验限价约束。 */
function assertFillPriceWithinLimit(order: Order, price: Decimal): void {
  if (order.type !== OrderType.Limit && order.type !== OrderType.StopLimit) return;
  const limitPrice = order.limitPrice;
  if (limitPrice === undefined) return;

  const isViolation =
    order.side === OrderSide.Buy ? price.compare(limitPrice) > 0 : price.compare(limitPrice) < 0;
  if (isViolation) {
    throw new OrderStateError(
      order.id,
      order.status,
      'fill',
      `成交价 ${price.toString()} 劣于限价 ${limitPrice.toString()}`,
    );
  }
}

/** 校验订单价格字段矩阵。 */
function assertOrderPriceMatrix(
  type: OrderType,
  limitPrice: Decimal | undefined,
  stopPrice: Decimal | undefined,
  instrument: Instrument,
): void {
  switch (type) {
    case OrderType.Market:
      if (limitPrice !== undefined || stopPrice !== undefined) {
        throw new FinanceDefinitionError('market 订单不能携带 limitPrice/stopPrice');
      }
      return;
    case OrderType.Limit:
      if (limitPrice === undefined) {
        throw new FinanceDefinitionError('limit 订单必须提供 limitPrice');
      }
      if (stopPrice !== undefined) {
        throw new FinanceDefinitionError('limit 订单不能携带 stopPrice');
      }
      assertPositiveFillPrice(instrument, limitPrice, 'order.limitPrice');
      return;
    case OrderType.Stop:
      if (stopPrice === undefined) {
        throw new FinanceDefinitionError('stop 订单必须提供 stopPrice');
      }
      if (limitPrice !== undefined) {
        throw new FinanceDefinitionError('stop 订单不能携带 limitPrice');
      }
      assertPositiveFillPrice(instrument, stopPrice, 'order.stopPrice');
      return;
    case OrderType.StopLimit:
      if (limitPrice === undefined || stopPrice === undefined) {
        throw new FinanceDefinitionError('stop-limit 订单必须同时提供 limitPrice 与 stopPrice');
      }
      assertPositiveFillPrice(instrument, limitPrice, 'order.limitPrice');
      assertPositiveFillPrice(instrument, stopPrice, 'order.stopPrice');
      return;
    default: {
      const exhaustive: never = type;
      throw new FinanceDefinitionError(`未知订单类型: ${String(exhaustive)}`);
    }
  }
}

/** 校验订单 id。 */
function assertOrderId(value: unknown, path: string): asserts value is string {
  assertNonEmptyString(value, path);
  if (!ORDER_ID_PATTERN.test(value)) {
    throw new FinanceDefinitionError(
      `${path} 必须匹配 ${ORDER_ID_PATTERN.source}（实际值: ${value}）`,
    );
  }
}

/** 校验正价格且满足 tick。 */
function assertPositiveFillPrice(instrument: Instrument, price: Decimal, path: string): void {
  if (!price.isPositive()) {
    throw new FinanceDefinitionError(`${path} 必须大于 0（实际值: ${price.toString()}）`);
  }
  if (!isPriceValid(instrument, price)) {
    throw new FinanceDefinitionError(
      `${path} 必须是 tickSize ${instrument.tickSize.toString()} 的整数倍（实际值: ${price.toString()}）`,
    );
  }
}

/** 校验正数量。 */
function assertPositiveFillQuantity(quantity: Decimal, path: string): void {
  if (!quantity.isPositive()) {
    throw new FinanceDefinitionError(`${path} 必须大于 0（实际值: ${quantity.toString()}）`);
  }
}

/** 校验状态。 */
function ensureStatus(order: Order, expected: OrderStatus, event: string): void {
  if (order.status !== expected) {
    throw new OrderStateError(order.id, order.status, event);
  }
}

/** 校验状态可撤销/过期。 */
function ensureActive(order: Order, event: string): void {
  if (!ACTIVE_STATUSES.has(order.status)) {
    throw new OrderStateError(order.id, order.status, event);
  }
}

/** 解析并校验事件时间。 */
function resolveEventTimestamp(order: Order, timestampMs: number | undefined): number {
  if (timestampMs === undefined) return order.updatedAtMs;
  if (!Number.isSafeInteger(timestampMs) || timestampMs < 0) {
    throw new OrderStateError(
      order.id,
      order.status,
      'timestamp',
      `timestampMs 必须是非负安全整数（实际值: ${String(timestampMs)}）`,
    );
  }
  if (timestampMs < order.updatedAtMs) {
    throw new OrderStateError(
      order.id,
      order.status,
      'timestamp',
      `事件时间 ${timestampMs} 早于订单更新时间 ${order.updatedAtMs}`,
    );
  }
  return timestampMs;
}

/**
 * 判断状态是否为终态（供外部持久化/适配器复用）。
 *
 * @param status - 订单状态
 * @returns 终态时 true
 */
export function isTerminalOrderStatus(status: OrderStatus): boolean {
  return TERMINAL_STATUSES.has(status);
}

/**
 * 判断状态是否合法。
 *
 * @param value - 待判断值
 * @returns 属于订单状态集合时 true
 */
export function isOrderStatus(value: unknown): value is OrderStatus {
  return typeof value === 'string' && ORDER_STATUSES.includes(value as OrderStatus);
}
