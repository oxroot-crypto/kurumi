/**
 * @file 撮合模拟单元测试：市价/限价/止损、盘口数量、FOK 与错误边界。
 * @description 覆盖 FC-SIM-*。
 * @module finance/simulation.test
 */

import { describe, expect, it } from 'vitest';

import { Decimal } from '@kurumi/math';
import { FinanceValueError } from './errors.js';
import { createQuote } from './market-data.js';
import {
  OrderSide,
  OrderType,
  TimeInForce,
  acceptOrder,
  cancelOrder,
  type Order,
} from './order.js';
import { isStopTriggered, matchOrderAgainstQuote } from './simulation.js';
import { T0, aapl, btcUsdt, makeOrder } from './fixtures.test-utils.js';

const instrument = btcUsdt();

function quote(ask = '100', bid = '99', askSize = '1', bidSize = '1') {
  return createQuote({
    instrument,
    bid: Decimal.parse(bid),
    ask: Decimal.parse(ask),
    bidSize: Decimal.parse(bidSize),
    askSize: Decimal.parse(askSize),
    timestampMs: T0,
  });
}

function open(overrides: Parameters<typeof makeOrder>[0] = {}): Order {
  return acceptOrder(makeOrder(overrides), T0 + 1);
}

function marketOrder(overrides: Parameters<typeof makeOrder>[0] = {}) {
  return open({
    type: OrderType.Market,
    limitPrice: undefined,
    ...overrides,
  });
}

describe('isStopTriggered', () => {
  it('buy stop 向上触发、sell stop 向下触发', () => {
    const buy = open({
      type: OrderType.Stop,
      limitPrice: undefined,
      stopPrice: Decimal.parse('100'),
    });
    expect(isStopTriggered(buy, Decimal.parse('100'))).toBe(true);
    expect(isStopTriggered(buy, Decimal.parse('101'))).toBe(true);
    expect(isStopTriggered(buy, Decimal.parse('99'))).toBe(false);

    const sell = open({
      type: OrderType.Stop,
      limitPrice: undefined,
      stopPrice: Decimal.parse('100'),
      side: OrderSide.Sell,
    });
    expect(isStopTriggered(sell, Decimal.parse('100'))).toBe(true);
    expect(isStopTriggered(sell, Decimal.parse('99'))).toBe(true);
    expect(isStopTriggered(sell, Decimal.parse('101'))).toBe(false);
  });

  it('非止损单或缺少 stopPrice 失败', () => {
    expect(() => isStopTriggered(open(), Decimal.ONE)).toThrow(FinanceValueError);
    const stopOrder = open({
      type: OrderType.Stop,
      limitPrice: undefined,
      stopPrice: Decimal.ONE,
    });
    const { stopPrice: _stopPrice, ...broken } = stopOrder;
    expect(() => isStopTriggered(broken, Decimal.ONE)).toThrow(FinanceValueError);
  });
});

describe('matchOrderAgainstQuote', () => {
  it('市价买入以 ask、市价卖出以 bid 成交', () => {
    const buy = matchOrderAgainstQuote(marketOrder(), quote('100', '99', '2', '2'));
    expect(buy?.fillPrice.toString()).toBe('100');
    expect(buy?.fillQuantity.toString()).toBe('1');

    const sell = matchOrderAgainstQuote(
      marketOrder({ side: OrderSide.Sell }),
      quote('100', '99', '2', '2'),
    );
    expect(sell?.fillPrice.toString()).toBe('99');
    expect(sell?.fillQuantity.toString()).toBe('1');
  });

  it('限价买卖只在可成交时撮合，且使用更优盘口价', () => {
    const limitBuy = open({ limitPrice: Decimal.parse('100.01') });
    expect(
      matchOrderAgainstQuote(limitBuy, quote('100', '99', '1', '1'))?.fillPrice.toString(),
    ).toBe('100');

    const notMarketableBuy = open({ limitPrice: Decimal.parse('99.99') });
    expect(matchOrderAgainstQuote(notMarketableBuy, quote('100', '99'))).toBeUndefined();

    const limitSell = open({ side: OrderSide.Sell, limitPrice: Decimal.parse('99') });
    expect(
      matchOrderAgainstQuote(limitSell, quote('100', '99', '1', '1'))?.fillPrice.toString(),
    ).toBe('99');

    const notMarketableSell = open({
      side: OrderSide.Sell,
      limitPrice: Decimal.parse('100'),
    });
    expect(matchOrderAgainstQuote(notMarketableSell, quote('100', '99'))).toBeUndefined();
  });

  it('stop 订单触发后才撮合，stop-limit 还需满足限价', () => {
    const stop = open({
      type: OrderType.Stop,
      limitPrice: undefined,
      stopPrice: Decimal.parse('100'),
    });
    expect(
      matchOrderAgainstQuote(stop, quote('100', '99'), {
        lastPrice: Decimal.parse('99'),
      }),
    ).toBeUndefined();
    expect(
      matchOrderAgainstQuote(stop, quote('100', '99', '1', '1'), {
        lastPrice: Decimal.parse('100'),
      })?.fillPrice.toString(),
    ).toBe('100');

    const stopLimit = open({
      type: OrderType.StopLimit,
      stopPrice: Decimal.parse('100'),
      limitPrice: Decimal.parse('100.5'),
    });
    expect(
      matchOrderAgainstQuote(stopLimit, quote('100.2', '99', '1', '1'), {
        lastPrice: Decimal.parse('100'),
      })?.fillPrice.toString(),
    ).toBe('100.2');
    expect(
      matchOrderAgainstQuote(stopLimit, quote('100.6', '99', '1', '1'), {
        lastPrice: Decimal.parse('100'),
      }),
    ).toBeUndefined();
  });

  it('受盘口数量与 maxQuantity 限制，FOK 不足则完全不成交', () => {
    const order = marketOrder({ quantity: Decimal.parse('2') });
    expect(
      matchOrderAgainstQuote(order, quote('100', '99', '1', '1'))?.fillQuantity.toString(),
    ).toBe('1');
    expect(
      matchOrderAgainstQuote(order, quote('100', '99', '2', '2'), {
        maxQuantity: Decimal.parse('0.5'),
      })?.fillQuantity.toString(),
    ).toBe('0.5');

    const fok = open({
      type: OrderType.Market,
      limitPrice: undefined,
      quantity: Decimal.parse('2'),
      timeInForce: TimeInForce.Fok,
    });
    expect(matchOrderAgainstQuote(fok, quote('100', '99', '1', '1'))).toBeUndefined();
    expect(matchOrderAgainstQuote(fok, quote('100', '99', '2', '2'))?.fillQuantity.toString()).toBe(
      '2',
    );

    const ioc = open({
      type: OrderType.Market,
      limitPrice: undefined,
      quantity: Decimal.parse('2'),
      timeInForce: TimeInForce.Ioc,
    });
    expect(matchOrderAgainstQuote(ioc, quote('100', '99', '1', '1'))?.fillQuantity.toString()).toBe(
      '1',
    );
  });

  it('零盘口数量不成交', () => {
    expect(matchOrderAgainstQuote(marketOrder(), quote('100', '99', '0', '0'))).toBeUndefined();
  });

  it('拒绝非 open、标的不一致、缺少 lastPrice 与非法 maxQuantity', () => {
    expect(() =>
      matchOrderAgainstQuote(makeOrder({ type: OrderType.Market, limitPrice: undefined }), quote()),
    ).toThrow(FinanceValueError);
    expect(() => matchOrderAgainstQuote(marketOrder(), quote())).not.toThrow();
    expect(() =>
      matchOrderAgainstQuote(
        marketOrder(),
        createQuote({
          instrument: aapl(),
          bid: Decimal.parse('100'),
          ask: Decimal.parse('101'),
          bidSize: Decimal.ONE,
          askSize: Decimal.ONE,
          timestampMs: T0,
        }),
      ),
    ).toThrow(FinanceValueError);
    const stop = open({
      type: OrderType.Stop,
      limitPrice: undefined,
      stopPrice: Decimal.parse('100'),
    });
    expect(() => matchOrderAgainstQuote(stop, quote())).toThrow(FinanceValueError);
    expect(() =>
      matchOrderAgainstQuote(marketOrder(), quote(), { maxQuantity: Decimal.ZERO }),
    ).toThrow(FinanceValueError);
    expect(() => matchOrderAgainstQuote(cancelOrder(open(), T0 + 2), quote())).toThrow(
      FinanceValueError,
    );
  });
});
