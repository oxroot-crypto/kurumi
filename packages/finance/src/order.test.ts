/**
 * @file 订单与成交单元测试。
 * @description 覆盖订单构造矩阵、合法/非法状态转移、超量/限价/时序与派生指标。
 * @module finance/order.test
 */

import { describe, expect, it } from 'vitest';

import { Decimal } from '@kurumi/math';
import {
  CurrencyMismatchError,
  FinanceDefinitionError,
  OrderOverfillError,
  OrderStateError,
} from './errors.js';
import { createMoney } from './money.js';
import {
  Liquidity,
  OrderSide,
  OrderStatus,
  OrderType,
  TimeInForce,
  acceptOrder,
  applyFill,
  applyOrderEvent,
  cancelOrder,
  createFill,
  expireOrder,
  isOrderOpen,
  isOrderStatus,
  isOrderTerminal,
  isTerminalOrderStatus,
  orderAverageFillPrice,
  orderFilledNotional,
  orderProgress,
  orderRemainingNotional,
  rejectOrder,
  type Order,
} from './order.js';
import { EUR, T0, btcUsdt, fillInput, makeFill, makeOrder } from './fixtures.test-utils.js';

function accepted(overrides: Parameters<typeof makeOrder>[0] = {}): Order {
  return acceptOrder(makeOrder(overrides), T0 + 1);
}

describe('createOrder', () => {
  it('创建 limit 订单（pending_new）', () => {
    const order = makeOrder();
    expect(order.status).toBe(OrderStatus.PendingNew);
    expect(order.filledQuantity.isZero()).toBe(true);
    expect(order.remainingQuantity.toString()).toBe('1');
    expect(order.timeInForce).toBe(TimeInForce.Gtc);
    expect(order.updatedAtMs).toBe(T0);
    expect(Object.isFrozen(order)).toBe(true);
  });

  it('支持 market/stop/stop-limit 与 clientOrderId', () => {
    const market = makeOrder({
      type: OrderType.Market,
      limitPrice: undefined,
      clientOrderId: 'c-1',
      timeInForce: TimeInForce.Ioc,
    });
    expect(market.limitPrice).toBeUndefined();
    expect(market.clientOrderId).toBe('c-1');

    const stop = makeOrder({
      type: OrderType.Stop,
      limitPrice: undefined,
      stopPrice: Decimal.parse('99'),
    });
    expect(stop.stopPrice?.toString()).toBe('99');

    const stopLimit = makeOrder({
      type: OrderType.StopLimit,
      limitPrice: Decimal.parse('100'),
      stopPrice: Decimal.parse('99'),
    });
    expect(stopLimit.limitPrice?.toString()).toBe('100');
    expect(stopLimit.stopPrice?.toString()).toBe('99');
  });

  it('拒绝非法 id/数量/时间/类型矩阵', () => {
    expect(() => makeOrder({ id: '' })).toThrow(FinanceDefinitionError);
    expect(() => makeOrder({ id: 'bad id' })).toThrow(FinanceDefinitionError);
    expect(() => makeOrder({ clientOrderId: 'bad id' })).toThrow(FinanceDefinitionError);
    expect(() => makeOrder({ quantity: Decimal.ZERO })).toThrow(FinanceDefinitionError);
    expect(() => makeOrder({ quantity: Decimal.parse('0.0005') })).toThrow(FinanceDefinitionError);
    expect(() => makeOrder({ createdAtMs: -1 })).toThrow(FinanceDefinitionError);
    expect(() => makeOrder({ type: 'iceberg' as never })).toThrow(FinanceDefinitionError);
    expect(() => makeOrder({ side: 'hold' as never })).toThrow(FinanceDefinitionError);
    expect(() => makeOrder({ timeInForce: 'week' as never })).toThrow(FinanceDefinitionError);
  });

  it('拒绝价格字段矩阵冲突与非法 tick', () => {
    expect(() => makeOrder({ type: OrderType.Market })).toThrow(FinanceDefinitionError);
    expect(() =>
      makeOrder({ type: OrderType.Market, limitPrice: undefined, stopPrice: Decimal.ONE }),
    ).toThrow(FinanceDefinitionError);
    expect(() => makeOrder({ type: OrderType.Limit, limitPrice: undefined })).toThrow(
      FinanceDefinitionError,
    );
    expect(() => makeOrder({ type: OrderType.Limit, stopPrice: Decimal.ONE })).toThrow(
      FinanceDefinitionError,
    );
    expect(() => makeOrder({ type: OrderType.Stop, limitPrice: undefined })).toThrow(
      FinanceDefinitionError,
    );
    expect(() =>
      makeOrder({ type: OrderType.Stop, limitPrice: undefined, stopPrice: undefined }),
    ).toThrow(FinanceDefinitionError);
    expect(() =>
      makeOrder({ type: OrderType.StopLimit, stopPrice: Decimal.ONE, limitPrice: undefined }),
    ).toThrow(FinanceDefinitionError);
    expect(() => makeOrder({ limitPrice: Decimal.parse('100.001') })).toThrow(
      FinanceDefinitionError,
    );
  });

  it('拒绝不可交易标的', () => {
    expect(() => makeOrder({ instrument: createOrderInstrument() })).toThrow(
      FinanceDefinitionError,
    );
  });
});

/** 创建不可交易标的的订单输入辅助（避免测试夹具污染）。 */
function createOrderInstrument() {
  const instrument = btcUsdt();
  return Object.freeze({ ...instrument, isTradable: false });
}

describe('createFill', () => {
  it('创建成交并支持手续费/流动性/tradeId', () => {
    const fill = createFill(
      fillInput({
        fee: createMoney(Decimal.parse('0.1'), btcUsdt().quoteCurrency),
        liquidity: Liquidity.Taker,
        tradeId: 't-1',
      }),
    );
    expect(fill.fee?.amount.toString()).toBe('0.1');
    expect(fill.liquidity).toBe(Liquidity.Taker);
    expect(fill.tradeId).toBe('t-1');
    expect(Object.isFrozen(fill)).toBe(true);
  });

  it('拒绝非法成交与手续费币种', () => {
    expect(() => createFill(fillInput({ id: '' }))).toThrow(FinanceDefinitionError);
    expect(() => createFill(fillInput({ orderId: 'bad id' }))).toThrow(FinanceDefinitionError);
    expect(() => createFill(fillInput({ price: Decimal.parse('100.001') }))).toThrow(
      FinanceDefinitionError,
    );
    expect(() => createFill(fillInput({ quantity: Decimal.ZERO }))).toThrow(FinanceDefinitionError);
    expect(() => createFill(fillInput({ timestampMs: -1 }))).toThrow(FinanceDefinitionError);
    expect(() => createFill(fillInput({ side: 'hold' as never }))).toThrow(FinanceDefinitionError);
    expect(() => createFill(fillInput({ liquidity: 'maker-taker' as never }))).toThrow(
      FinanceDefinitionError,
    );
    expect(() => createFill(fillInput({ tradeId: ' ' }))).toThrow(FinanceDefinitionError);
    expect(() => createFill(fillInput({ fee: createMoney(Decimal.ONE, EUR) }))).toThrow(
      CurrencyMismatchError,
    );
  });
});

describe('订单状态机', () => {
  it('pending_new -> open -> partially_filled -> filled', () => {
    const pending = makeOrder({ type: OrderType.Market, limitPrice: undefined });
    const open = acceptOrder(pending, T0 + 1);
    expect(open.status).toBe(OrderStatus.Open);

    const partial = applyFill(
      open,
      makeFill({
        price: Decimal.parse('100'),
        quantity: Decimal.parse('0.4'),
        timestampMs: T0 + 2,
      }),
    );
    expect(partial.status).toBe(OrderStatus.PartiallyFilled);
    expect(partial.filledQuantity.toString()).toBe('0.4');
    expect(partial.remainingQuantity.toString()).toBe('0.6');

    const filled = applyFill(
      partial,
      makeFill({
        id: 'f-2',
        price: Decimal.parse('101'),
        quantity: Decimal.parse('0.6'),
        timestampMs: T0 + 3,
      }),
    );
    expect(filled.status).toBe(OrderStatus.Filled);
    expect(filled.remainingQuantity.isZero()).toBe(true);
    expect(filled.filledNotional.toString()).toBe('100.6');
    expect(orderAverageFillPrice(filled)?.toString()).toBe('100.6');
    expect(orderProgress(filled).toString()).toBe('1');
  });

  it('pending_new -> rejected / canceled / expired', () => {
    const rejected = rejectOrder(makeOrder(), 'risk', T0 + 1);
    expect(rejected.status).toBe(OrderStatus.Rejected);
    expect(rejected.rejectReason).toBe('risk');

    const canceled = cancelOrder(makeOrder(), T0 + 1);
    expect(canceled.status).toBe(OrderStatus.Canceled);

    const expired = expireOrder(makeOrder(), T0 + 1);
    expect(expired.status).toBe(OrderStatus.Expired);
  });

  it('open/partially_filled 可撤销与过期', () => {
    const open = accepted();
    expect(cancelOrder(open, T0 + 2).status).toBe(OrderStatus.Canceled);
    expect(expireOrder(open, T0 + 2).status).toBe(OrderStatus.Expired);

    const partial = applyFill(
      open,
      makeFill({ quantity: Decimal.parse('0.5'), timestampMs: T0 + 2 }),
    );
    expect(cancelOrder(partial, T0 + 3).status).toBe(OrderStatus.Canceled);
    expect(expireOrder(partial, T0 + 3).status).toBe(OrderStatus.Expired);
  });

  it('终态不再接受任何事件', () => {
    const filled = applyFill(accepted(), makeFill({ quantity: Decimal.ONE, timestampMs: T0 + 2 }));
    const terminalOrders = [
      filled,
      cancelOrder(accepted(), T0 + 2),
      rejectOrder(makeOrder(), 'risk', T0 + 1),
      expireOrder(accepted(), T0 + 2),
    ];
    for (const order of terminalOrders) {
      expect(isOrderTerminal(order)).toBe(true);
      expect(isTerminalOrderStatus(order.status)).toBe(true);
      expect(() => acceptOrder(order, T0 + 3)).toThrow(OrderStateError);
      expect(() => rejectOrder(order, 'again', T0 + 3)).toThrow(OrderStateError);
      expect(() => cancelOrder(order, T0 + 3)).toThrow(OrderStateError);
      expect(() => expireOrder(order, T0 + 3)).toThrow(OrderStateError);
      expect(() => applyFill(order, makeFill({ timestampMs: T0 + 3 }))).toThrow(OrderStateError);
    }
  });

  it('pending_new 不能直接成交', () => {
    expect(() => applyOrderEvent(makeOrder(), { type: 'fill', fill: makeFill() })).toThrow(
      OrderStateError,
    );
  });

  it('事件时间倒退失败', () => {
    expect(() => acceptOrder(makeOrder(), T0 - 1)).toThrow(OrderStateError);
    expect(() => cancelOrder(accepted(), T0)).toThrow(OrderStateError);
    expect(() => applyFill(accepted(), makeFill({ timestampMs: T0 }))).toThrow(OrderStateError);
  });

  it('拒绝空 reject 原因与未知事件', () => {
    expect(() => rejectOrder(makeOrder(), ' ', T0 + 1)).toThrow(FinanceDefinitionError);
    expect(() => applyOrderEvent(makeOrder(), { type: 'unknown' } as never)).toThrow(
      OrderStateError,
    );
  });
});

describe('成交约束', () => {
  it('拒绝 orderId/instrument/side 不一致', () => {
    const order = accepted();
    expect(() => applyFill(order, makeFill({ orderId: 'other' }))).toThrow(OrderStateError);
    expect(() => applyFill(order, makeFill({ instrument: createOtherInstrument() }))).toThrow(
      OrderStateError,
    );
    expect(() => applyFill(order, makeFill({ side: OrderSide.Sell }))).toThrow(OrderStateError);
  });

  it('拒绝超量成交且保持原订单不变', () => {
    const order = accepted();
    expect(() => applyFill(order, makeFill({ quantity: Decimal.parse('2') }))).toThrow(
      OrderOverfillError,
    );
    expect(order.filledQuantity.isZero()).toBe(true);
  });

  it('拒绝限价违规', () => {
    const order = accepted();
    expect(() => applyFill(order, makeFill({ price: Decimal.parse('101') }))).toThrow(
      OrderStateError,
    );
    const sell = accepted({ side: OrderSide.Sell, limitPrice: Decimal.parse('100') });
    expect(() =>
      applyFill(sell, makeFill({ side: OrderSide.Sell, price: Decimal.parse('99') })),
    ).toThrow(OrderStateError);
  });

  it('拒绝非法 tick 与负/零数量', () => {
    const order = accepted();
    const validFill = makeFill();
    expect(() => applyFill(order, { ...validFill, price: Decimal.parse('100.001') })).toThrow(
      OrderStateError,
    );
    expect(() => applyFill(order, { ...validFill, quantity: Decimal.ZERO })).toThrow(
      OrderStateError,
    );
  });

  it('拒绝非法成交时间与手续费币种，避免污染订单时间', () => {
    const order = accepted();
    const validFill = makeFill();
    expect(() => applyFill(order, { ...validFill, timestampMs: Number.NaN })).toThrow(
      OrderStateError,
    );
    expect(() =>
      applyFill(order, { ...validFill, timestampMs: Number.MAX_SAFE_INTEGER + 1 }),
    ).toThrow(OrderStateError);
    expect(() => applyFill(order, { ...validFill, fee: createMoney(Decimal.ONE, EUR) })).toThrow(
      CurrencyMismatchError,
    );
  });
});

describe('订单派生指标', () => {
  it('平均值/名义价值/剩余名义/进度', () => {
    const pending = makeOrder({ limitPrice: Decimal.parse('200') });
    expect(orderAverageFillPrice(pending)).toBeUndefined();
    const open = acceptOrder(pending, T0 + 1);
    const partial = applyFill(
      open,
      makeFill({
        price: Decimal.parse('100'),
        quantity: Decimal.parse('0.5'),
        timestampMs: T0 + 2,
      }),
    );
    const filled = applyFill(
      partial,
      makeFill({
        id: 'f-2',
        price: Decimal.parse('110'),
        quantity: Decimal.parse('0.5'),
        timestampMs: T0 + 3,
      }),
    );
    expect(orderAverageFillPrice(filled)?.toString()).toBe('105');
    expect(orderFilledNotional(filled).amount.toString()).toBe('105');
    expect(orderRemainingNotional(filled, Decimal.parse('120')).amount.toString()).toBe('0');
    expect(orderProgress(partial).toString()).toBe('0.5');
  });

  it('isOrderOpen / isOrderStatus', () => {
    const pending = makeOrder();
    const open = acceptOrder(pending, T0 + 1);
    expect(isOrderOpen(open)).toBe(true);
    expect(isOrderOpen(pending)).toBe(false);
    expect(isOrderStatus(OrderStatus.Open)).toBe(true);
    expect(isOrderStatus('unknown')).toBe(false);
  });
});

/** 另一个 key 的标的。 */
function createOtherInstrument() {
  return Object.freeze({ ...btcUsdt(), symbol: 'ETH/USDT' });
}
