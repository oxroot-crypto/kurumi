/**
 * @file 行情单元测试：报价、成交、K 线、Ticker、盘口与增量盘口。
 * @description 覆盖 FC-MD-* 全部约束、边界与序列错误。
 * @module finance/market-data.test
 */

import { describe, expect, it } from 'vitest';

import { Decimal } from '@kurumi/math';
import { MarketDataValidationError } from './errors.js';
import {
  BarInterval,
  BookSide,
  applyOrderBookDelta,
  barIntervalMs,
  barIsBullish,
  barNotional,
  barRange,
  barTypicalPrice,
  bookBestAsk,
  bookBestBid,
  bookDepth,
  bookDepthNotional,
  bookImbalance,
  bookMidPrice,
  bookSpread,
  bookSpreadBps,
  bookTotalNotional,
  createBar,
  createOrderBook,
  createOrderBookDelta,
  createQuote,
  createTicker,
  createTrade,
  quoteMid,
  quoteSpread,
  quoteSpreadBps,
  tradeNotional,
  type OrderBookLevel,
} from './market-data.js';
import { createMoney } from './money.js';
import { T0, aapl, btcUsdt } from './fixtures.test-utils.js';

const instrument = btcUsdt();

function level(price: string, quantity: string): OrderBookLevel {
  return { price: Decimal.parse(price), quantity: Decimal.parse(quantity) };
}

describe('Quote', () => {
  it('创建并计算 mid/spread/bps', () => {
    const quote = createQuote({
      instrument,
      bid: Decimal.parse('100.01'),
      ask: Decimal.parse('100.03'),
      bidSize: Decimal.parse('1'),
      askSize: Decimal.parse('2'),
      timestampMs: T0,
    });
    expect(quoteMid(quote).toString()).toBe('100.02');
    expect(quoteSpread(quote).toString()).toBe('0.02');
    expect(quoteSpreadBps(quote).toString()).toBe('1.99960008');
    expect(Object.isFrozen(quote)).toBe(true);
  });

  it('拒绝交叉/非法价格、数量与时间', () => {
    const base = {
      instrument,
      bid: Decimal.parse('100'),
      ask: Decimal.parse('101'),
      bidSize: Decimal.ONE,
      askSize: Decimal.ONE,
      timestampMs: T0,
    };
    expect(() => createQuote({ ...base, bid: Decimal.parse('101') })).toThrow(
      MarketDataValidationError,
    );
    expect(() => createQuote({ ...base, bid: Decimal.parse('100.001') })).toThrow(
      MarketDataValidationError,
    );
    expect(() => createQuote({ ...base, bidSize: Decimal.parse('-1') })).toThrow(
      MarketDataValidationError,
    );
    expect(() => createQuote({ ...base, timestampMs: -1 })).toThrow(MarketDataValidationError);
    expect(() => createQuote({ ...base, instrument: {} as never })).toThrow(
      MarketDataValidationError,
    );
  });
});

describe('Trade', () => {
  it('创建成交与名义价值', () => {
    const trade = createTrade({
      instrument,
      price: Decimal.parse('100'),
      quantity: Decimal.parse('2'),
      side: 'buy',
      timestampMs: T0,
      tradeId: 't-1',
    });
    expect(trade.tradeId).toBe('t-1');
    expect(tradeNotional(trade).amount.toString()).toBe('200');
    expect(Object.isFrozen(trade)).toBe(true);
  });

  it('拒绝非法字段', () => {
    const base = {
      instrument,
      price: Decimal.parse('100'),
      quantity: Decimal.ONE,
      side: 'buy' as const,
      timestampMs: T0,
    };
    expect(() => createTrade({ ...base, price: Decimal.parse('100.001') })).toThrow(
      MarketDataValidationError,
    );
    expect(() => createTrade({ ...base, quantity: Decimal.ZERO })).toThrow(
      MarketDataValidationError,
    );
    expect(() => createTrade({ ...base, side: 'hold' as never })).toThrow(
      MarketDataValidationError,
    );
    expect(() => createTrade({ ...base, timestampMs: -1 })).toThrow(MarketDataValidationError);
    expect(() => createTrade({ ...base, tradeId: ' ' })).toThrow(MarketDataValidationError);
  });
});

describe('Bar', () => {
  const bar = createBar({
    instrument,
    interval: BarInterval.Minute1,
    openTimeMs: T0,
    closeTimeMs: T0 + 60_000,
    open: Decimal.parse('100'),
    high: Decimal.parse('105'),
    low: Decimal.parse('99'),
    close: Decimal.parse('104'),
    volume: Decimal.parse('10'),
    tradeCount: 5,
  });

  it('创建 K 线并计算派生指标', () => {
    expect(bar.tradeCount).toBe(5);
    expect(barRange(bar).toString()).toBe('6');
    expect(barTypicalPrice(bar).toString()).toBe('102.666666666666666667');
    expect(barNotional(bar).amount.toString()).toBe('1026.66666666666666667');
    expect(barIsBullish(bar)).toBe(true);
    expect(barIntervalMs(BarInterval.Week1)).toBe(604_800_000);
    expect(Object.isFrozen(bar)).toBe(true);
  });

  it('拒绝非法 OHLC、周期与时间区间', () => {
    const base = {
      instrument,
      interval: BarInterval.Minute1,
      openTimeMs: T0,
      closeTimeMs: T0 + 60_000,
      open: Decimal.parse('100'),
      high: Decimal.parse('105'),
      low: Decimal.parse('99'),
      close: Decimal.parse('104'),
      volume: Decimal.parse('10'),
    };
    expect(() => createBar({ ...base, high: Decimal.parse('99') })).toThrow(
      MarketDataValidationError,
    );
    expect(() => createBar({ ...base, low: Decimal.parse('101') })).toThrow(
      MarketDataValidationError,
    );
    expect(() => createBar({ ...base, high: Decimal.parse('98') })).toThrow(
      MarketDataValidationError,
    );
    expect(() => createBar({ ...base, closeTimeMs: T0 + 60_001 })).toThrow(
      MarketDataValidationError,
    );
    expect(() => createBar({ ...base, open: Decimal.parse('100.001') })).toThrow(
      MarketDataValidationError,
    );
    expect(() => createBar({ ...base, volume: Decimal.parse('-1') })).toThrow(
      MarketDataValidationError,
    );
    expect(() => createBar({ ...base, tradeCount: -1 })).toThrow(MarketDataValidationError);
    expect(() => barIntervalMs('2m' as never)).toThrow(MarketDataValidationError);
  });
});

describe('Ticker', () => {
  it('创建并允许可选字段', () => {
    const ticker = createTicker({
      instrument,
      lastPrice: Decimal.parse('100'),
      bid: Decimal.parse('99.99'),
      ask: Decimal.parse('100.01'),
      markPrice: Decimal.parse('100'),
      volume24h: Decimal.parse('1000'),
      timestampMs: T0,
    });
    expect(ticker.volume24h?.toString()).toBe('1000');
    expect(Object.isFrozen(ticker)).toBe(true);
  });

  it('拒绝交叉/非法 ticker', () => {
    const base = {
      instrument,
      lastPrice: Decimal.parse('100'),
      timestampMs: T0,
    };
    expect(() =>
      createTicker({
        ...base,
        bid: Decimal.parse('100.02'),
        ask: Decimal.parse('100.01'),
      }),
    ).toThrow(MarketDataValidationError);
    expect(() => createTicker({ ...base, lastPrice: Decimal.parse('100.001') })).toThrow(
      MarketDataValidationError,
    );
    expect(() => createTicker({ ...base, markPrice: Decimal.ZERO })).toThrow(
      MarketDataValidationError,
    );
    expect(() => createTicker({ ...base, volume24h: Decimal.parse('-1') })).toThrow(
      MarketDataValidationError,
    );
  });
});

describe('OrderBook', () => {
  const book = createOrderBook({
    instrument,
    bids: [level('100', '1'), level('99.5', '2')],
    asks: [level('100.5', '3'), level('101', '4')],
    timestampMs: T0,
    sequence: 1,
  });

  it('创建有序盘口并计算派生指标', () => {
    expect(bookBestBid(book)?.toString()).toBe('100');
    expect(bookBestAsk(book)?.toString()).toBe('100.5');
    expect(bookMidPrice(book)?.toString()).toBe('100.25');
    expect(bookSpread(book)?.toString()).toBe('0.5');
    expect(bookSpreadBps(book)?.toString()).toBe('49.87531172');
    expect(bookDepth(book, BookSide.Bids).toString()).toBe('3');
    expect(bookDepthNotional(book, BookSide.Bids).amount.toString()).toBe('299');
    expect(bookImbalance(book).toString()).toBe('-0.4');
    expect(bookTotalNotional(book).amount.toString()).toBe('1004.5');
    expect(Object.isFrozen(book)).toBe(true);
    expect(Object.isFrozen(book.bids)).toBe(true);
  });

  it('空盘口合法且派生指标为零/undefined', () => {
    const empty = createOrderBook({
      instrument,
      bids: [],
      asks: [],
      timestampMs: T0,
      sequence: 0,
    });
    expect(bookBestBid(empty)).toBeUndefined();
    expect(bookBestAsk(empty)).toBeUndefined();
    expect(bookMidPrice(empty)).toBeUndefined();
    expect(bookSpread(empty)).toBeUndefined();
    expect(bookSpreadBps(empty)).toBeUndefined();
    expect(bookDepth(empty, BookSide.Bids).isZero()).toBe(true);
    expect(bookDepthNotional(empty, BookSide.Bids).amount.isZero()).toBe(true);
    expect(bookImbalance(empty).isZero()).toBe(true);
    expect(bookTotalNotional(empty).amount.isZero()).toBe(true);
  });

  it('拒绝乱序、重复、交叉、非法数量与非法 tick', () => {
    const base = {
      instrument,
      bids: [level('100', '1')],
      asks: [level('101', '1')],
      timestampMs: T0,
      sequence: 1,
    };
    expect(() => createOrderBook({ ...base, bids: [level('99', '1'), level('100', '1')] })).toThrow(
      MarketDataValidationError,
    );
    expect(() =>
      createOrderBook({ ...base, bids: [level('100', '1'), level('100', '2')] }),
    ).toThrow(MarketDataValidationError);
    expect(() =>
      createOrderBook({ ...base, bids: [level('101', '1')], asks: [level('101', '1')] }),
    ).toThrow(MarketDataValidationError);
    expect(() => createOrderBook({ ...base, bids: [level('100', '0')] })).toThrow(
      MarketDataValidationError,
    );
    expect(() => createOrderBook({ ...base, bids: [level('100.001', '1')] })).toThrow(
      MarketDataValidationError,
    );
  });

  it('创建并应用增量盘口（更新/新增/删除）', () => {
    const delta = createOrderBookDelta({
      instrument,
      bids: [level('100', '5'), level('99', '1')],
      asks: [level('100.5', '0'), level('100.6', '2')],
      timestampMs: T0 + 1,
      sequence: 2,
    });
    const next = applyOrderBookDelta(book, delta);
    expect(next.sequence).toBe(2);
    expect(next.bids.map((item) => item.price.toString())).toEqual(['100', '99.5', '99']);
    expect(next.bids[0]?.quantity.toString()).toBe('5');
    expect(next.asks.map((item) => item.price.toString())).toEqual(['100.6', '101']);
    expect(book.sequence).toBe(1);
    expect(book.bids[0]?.quantity.toString()).toBe('1');
  });

  it('拒绝增量序列跳号、标的不一致、重复/负数量', () => {
    const delta = createOrderBookDelta({
      instrument,
      bids: [level('100', '1')],
      asks: [],
      timestampMs: T0 + 1,
      sequence: 2,
    });
    expect(() => applyOrderBookDelta(book, { ...delta, sequence: 3 })).toThrow(
      MarketDataValidationError,
    );
    expect(() => applyOrderBookDelta(book, { ...delta, instrument: aapl() } as never)).toThrow(
      MarketDataValidationError,
    );
    expect(() =>
      createOrderBookDelta({
        instrument,
        bids: [level('100', '1'), level('100', '2')],
        asks: [],
        timestampMs: T0 + 1,
        sequence: 2,
      }),
    ).toThrow(MarketDataValidationError);
    expect(() =>
      createOrderBookDelta({
        instrument,
        bids: [level('100', '-1')],
        asks: [],
        timestampMs: T0 + 1,
        sequence: 2,
      }),
    ).toThrow(MarketDataValidationError);
  });

  it('增量应用后交叉盘口失败且不产生新快照', () => {
    const delta = createOrderBookDelta({
      instrument,
      bids: [level('101', '1')],
      asks: [],
      timestampMs: T0 + 1,
      sequence: 2,
    });
    expect(() => applyOrderBookDelta(book, delta)).toThrow(MarketDataValidationError);
  });

  it('bookDepthNotional 对单侧空盘口返回零', () => {
    const oneSided = createOrderBook({
      instrument,
      bids: [level('100', '1')],
      asks: [],
      timestampMs: T0,
      sequence: 0,
    });
    expect(bookDepthNotional(oneSided, BookSide.Asks).amount.toString()).toBe('0');
    expect(bookMidPrice(oneSided)).toBeUndefined();
  });

  it('构造 quote 货币手续费金额（确保 Money 工具联动）', () => {
    expect(createMoney(Decimal.ONE, instrument.quoteCurrency).currency.code).toBe('USDT');
  });
});
