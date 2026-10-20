/**
 * @file 行情数据：报价、成交、K 线、Ticker、盘口快照与增量盘口。
 * @description 所有行情在构造/应用边界校验价格 tick、数量、排序、交叉盘口与序列连续性，并递归冻结。
 * @module finance/market-data
 */

import { isPlainObject } from '@kurumi/core';

import { Decimal } from '@kurumi/math';
import { MarketDataValidationError } from './errors.js';
import {
  instrumentKey,
  isInstrument,
  isPriceValid,
  notionalMoney,
  sameInstrument,
  type Instrument,
} from './instrument.js';
import { createMoney, sumMoney, type Money } from './money.js';
import {
  assertArrayMaxLength,
  assertNonEmptyString,
  assertNonNegativeDecimal,
  assertOneOf,
  assertPositiveDecimal,
  assertSafeInteger,
  assertTimestampMs,
  type ValidationErrorFactory,
} from './validation.js';

/** 单侧盘口最大档位，防止恶意超大盘口。 */
export const MAX_ORDER_BOOK_LEVELS = 1000;

/** 行情错误工厂。 */
const marketDataError: ValidationErrorFactory = (message) => new MarketDataValidationError(message);

/** 成交主动方。 */
export const TradeSide = {
  /** 买方主动。 */
  Buy: 'buy',
  /** 卖方主动。 */
  Sell: 'sell',
  /** 未知。 */
  Unknown: 'unknown',
} as const;

/** 成交主动方联合类型。 */
export type TradeSide = (typeof TradeSide)[keyof typeof TradeSide];

/** 成交主动方集合。 */
const TRADE_SIDES: readonly TradeSide[] = Object.freeze(Object.values(TradeSide));

/** 盘口侧。 */
export const BookSide = {
  /** 买盘。 */
  Bids: 'bids',
  /** 卖盘。 */
  Asks: 'asks',
} as const;

/** 盘口侧联合类型。 */
export type BookSide = (typeof BookSide)[keyof typeof BookSide];

/** K 线周期。 */
export const BarInterval = {
  Minute1: '1m',
  Minute3: '3m',
  Minute5: '5m',
  Minute15: '15m',
  Minute30: '30m',
  Hour1: '1h',
  Hour2: '2h',
  Hour4: '4h',
  Hour6: '6h',
  Hour8: '8h',
  Hour12: '12h',
  Day1: '1d',
  Day3: '3d',
  Week1: '1w',
} as const;

/** K 线周期联合类型。 */
export type BarInterval = (typeof BarInterval)[keyof typeof BarInterval];

/** 全部 K 线周期。 */
const BAR_INTERVALS: readonly BarInterval[] = Object.freeze(Object.values(BarInterval));

/** 周期毫秒映射。 */
const BAR_INTERVAL_MS: Readonly<Record<BarInterval, number>> = Object.freeze({
  [BarInterval.Minute1]: 60_000,
  [BarInterval.Minute3]: 3 * 60_000,
  [BarInterval.Minute5]: 5 * 60_000,
  [BarInterval.Minute15]: 15 * 60_000,
  [BarInterval.Minute30]: 30 * 60_000,
  [BarInterval.Hour1]: 60 * 60_000,
  [BarInterval.Hour2]: 2 * 60 * 60_000,
  [BarInterval.Hour4]: 4 * 60 * 60_000,
  [BarInterval.Hour6]: 6 * 60 * 60_000,
  [BarInterval.Hour8]: 8 * 60 * 60_000,
  [BarInterval.Hour12]: 12 * 60 * 60_000,
  [BarInterval.Day1]: 24 * 60 * 60_000,
  [BarInterval.Day3]: 3 * 24 * 60 * 60_000,
  [BarInterval.Week1]: 7 * 24 * 60 * 60_000,
});

/** 报价。 */
export interface Quote {
  /** 标的。 */
  readonly instrument: Instrument;
  /** 买一价。 */
  readonly bid: Decimal;
  /** 卖一价。 */
  readonly ask: Decimal;
  /** 买一量。 */
  readonly bidSize: Decimal;
  /** 卖一量。 */
  readonly askSize: Decimal;
  /** 行情时间（epoch 毫秒）。 */
  readonly timestampMs: number;
}

/** 报价输入。 */
export interface CreateQuoteInput {
  /** 标的。 */
  readonly instrument: Instrument;
  /** 买一价。 */
  readonly bid: Decimal;
  /** 卖一价。 */
  readonly ask: Decimal;
  /** 买一量。 */
  readonly bidSize: Decimal;
  /** 卖一量。 */
  readonly askSize: Decimal;
  /** 行情时间（epoch 毫秒）。 */
  readonly timestampMs: number;
}

/** 成交。 */
export interface Trade {
  /** 标的。 */
  readonly instrument: Instrument;
  /** 成交价。 */
  readonly price: Decimal;
  /** 成交量。 */
  readonly quantity: Decimal;
  /** 主动方。 */
  readonly side: TradeSide;
  /** 成交时间（epoch 毫秒）。 */
  readonly timestampMs: number;
  /** 交易所成交 id。 */
  readonly tradeId?: string;
}

/** 成交输入。 */
export interface CreateTradeInput {
  /** 标的。 */
  readonly instrument: Instrument;
  /** 成交价。 */
  readonly price: Decimal;
  /** 成交量。 */
  readonly quantity: Decimal;
  /** 主动方。 */
  readonly side: TradeSide;
  /** 成交时间（epoch 毫秒）。 */
  readonly timestampMs: number;
  /** 交易所成交 id。 */
  readonly tradeId?: string;
}

/** OHLCV K 线。 */
export interface Bar {
  /** 标的。 */
  readonly instrument: Instrument;
  /** 周期。 */
  readonly interval: BarInterval;
  /** 开盘时间（epoch 毫秒）。 */
  readonly openTimeMs: number;
  /** 收盘时间（epoch 毫秒，开区间）。 */
  readonly closeTimeMs: number;
  /** 开盘价。 */
  readonly open: Decimal;
  /** 最高价。 */
  readonly high: Decimal;
  /** 最低价。 */
  readonly low: Decimal;
  /** 收盘价。 */
  readonly close: Decimal;
  /** 成交量。 */
  readonly volume: Decimal;
  /** 成交笔数。 */
  readonly tradeCount?: number;
}

/** K 线输入。 */
export interface CreateBarInput {
  /** 标的。 */
  readonly instrument: Instrument;
  /** 周期。 */
  readonly interval: BarInterval;
  /** 开盘时间（epoch 毫秒）。 */
  readonly openTimeMs: number;
  /** 收盘时间（epoch 毫秒）。 */
  readonly closeTimeMs: number;
  /** 开盘价。 */
  readonly open: Decimal;
  /** 最高价。 */
  readonly high: Decimal;
  /** 最低价。 */
  readonly low: Decimal;
  /** 收盘价。 */
  readonly close: Decimal;
  /** 成交量。 */
  readonly volume: Decimal;
  /** 成交笔数。 */
  readonly tradeCount?: number;
}

/** Ticker 快照。 */
export interface Ticker {
  /** 标的。 */
  readonly instrument: Instrument;
  /** 最新成交价。 */
  readonly lastPrice: Decimal;
  /** 买一价。 */
  readonly bid?: Decimal;
  /** 卖一价。 */
  readonly ask?: Decimal;
  /** 标记价格。 */
  readonly markPrice?: Decimal;
  /** 24 小时成交量。 */
  readonly volume24h?: Decimal;
  /** 时间（epoch 毫秒）。 */
  readonly timestampMs: number;
}

/** Ticker 输入。 */
export interface CreateTickerInput {
  /** 标的。 */
  readonly instrument: Instrument;
  /** 最新成交价。 */
  readonly lastPrice: Decimal;
  /** 买一价。 */
  readonly bid?: Decimal;
  /** 卖一价。 */
  readonly ask?: Decimal;
  /** 标记价格。 */
  readonly markPrice?: Decimal;
  /** 24 小时成交量。 */
  readonly volume24h?: Decimal;
  /** 时间（epoch 毫秒）。 */
  readonly timestampMs: number;
}

/** 盘口档位。 */
export interface OrderBookLevel {
  /** 价格。 */
  readonly price: Decimal;
  /** 数量。 */
  readonly quantity: Decimal;
}

/** 盘口快照。 */
export interface OrderBookSnapshot {
  /** 标的。 */
  readonly instrument: Instrument;
  /** 买盘，价格严格递减。 */
  readonly bids: readonly OrderBookLevel[];
  /** 卖盘，价格严格递增。 */
  readonly asks: readonly OrderBookLevel[];
  /** 快照时间（epoch 毫秒）。 */
  readonly timestampMs: number;
  /** 序列号。 */
  readonly sequence: number;
}

/** 盘口快照输入。 */
export interface CreateOrderBookInput {
  /** 标的。 */
  readonly instrument: Instrument;
  /** 买盘。 */
  readonly bids: readonly OrderBookLevel[];
  /** 卖盘。 */
  readonly asks: readonly OrderBookLevel[];
  /** 快照时间（epoch 毫秒）。 */
  readonly timestampMs: number;
  /** 序列号。 */
  readonly sequence: number;
}

/** 盘口增量。 */
export interface OrderBookDelta {
  /** 标的。 */
  readonly instrument: Instrument;
  /** 买盘更新；quantity 为 0 表示删除。 */
  readonly bids: readonly OrderBookLevel[];
  /** 卖盘更新；quantity 为 0 表示删除。 */
  readonly asks: readonly OrderBookLevel[];
  /** 更新时间（epoch 毫秒）。 */
  readonly timestampMs: number;
  /** 序列号，必须等于快照序列号 + 1。 */
  readonly sequence: number;
}

/** 盘口增量输入。 */
export interface CreateOrderBookDeltaInput {
  /** 标的。 */
  readonly instrument: Instrument;
  /** 买盘更新。 */
  readonly bids: readonly OrderBookLevel[];
  /** 卖盘更新。 */
  readonly asks: readonly OrderBookLevel[];
  /** 更新时间（epoch 毫秒）。 */
  readonly timestampMs: number;
  /** 序列号。 */
  readonly sequence: number;
}

/**
 * 获取 K 线周期毫秒数。
 *
 * @param interval - 周期
 * @returns 毫秒
 * @throws {MarketDataValidationError} 周期非法
 */
export function barIntervalMs(interval: BarInterval): number {
  assertOneOf(interval, BAR_INTERVALS, 'interval', marketDataError);
  return BAR_INTERVAL_MS[interval];
}

/**
 * 创建报价。
 *
 * @param input - 买一/卖一价格、数量与时间
 * @returns 冻结报价
 * @throws {MarketDataValidationError} 价格/数量/时间或交叉盘口非法
 */
export function createQuote(input: CreateQuoteInput): Quote {
  assertInstrumentValue(input.instrument, 'quote.instrument');
  assertPositiveDecimal(input.bid, 'quote.bid', marketDataError);
  assertPositiveDecimal(input.ask, 'quote.ask', marketDataError);
  assertNonNegativeDecimal(input.bidSize, 'quote.bidSize', marketDataError);
  assertNonNegativeDecimal(input.askSize, 'quote.askSize', marketDataError);
  assertTimestampMs(input.timestampMs, 'quote.timestampMs', marketDataError);
  assertMarketPrice(input.instrument, input.bid, 'quote.bid');
  assertMarketPrice(input.instrument, input.ask, 'quote.ask');
  if (input.bid.compare(input.ask) >= 0) {
    throw new MarketDataValidationError(
      `quote.bid 必须小于 ask（bid=${input.bid.toString()}, ask=${input.ask.toString()}）`,
    );
  }
  return Object.freeze({
    instrument: input.instrument,
    bid: input.bid,
    ask: input.ask,
    bidSize: input.bidSize,
    askSize: input.askSize,
    timestampMs: input.timestampMs,
  });
}

/**
 * 计算报价中间价。
 *
 * @param quote - 报价
 * @returns `(bid + ask) / 2`，scale 18、half-even
 */
export function quoteMid(quote: Quote): Decimal {
  return quote.bid.add(quote.ask).divide(Decimal.fromInteger(2));
}

/**
 * 计算买卖价差。
 *
 * @param quote - 报价
 * @returns `ask - bid`
 */
export function quoteSpread(quote: Quote): Decimal {
  return quote.ask.subtract(quote.bid);
}

/**
 * 计算买卖价差（基点）。
 *
 * @param quote - 报价
 * @returns `spread / mid * 10000`，scale 8、half-even
 */
export function quoteSpreadBps(quote: Quote): Decimal {
  const mid = quoteMid(quote);
  return quoteSpread(quote)
    .multiply(Decimal.fromInteger(10_000))
    .divide(mid, { scale: 8, rounding: 'half-even' });
}

/**
 * 创建成交。
 *
 * @param input - 成交价、数量、主动方与时间
 * @returns 冻结成交
 * @throws {MarketDataValidationError} 字段非法
 */
export function createTrade(input: CreateTradeInput): Trade {
  assertInstrumentValue(input.instrument, 'trade.instrument');
  assertPositiveDecimal(input.price, 'trade.price', marketDataError);
  assertPositiveDecimal(input.quantity, 'trade.quantity', marketDataError);
  assertOneOf(input.side, TRADE_SIDES, 'trade.side', marketDataError);
  assertTimestampMs(input.timestampMs, 'trade.timestampMs', marketDataError);
  assertMarketPrice(input.instrument, input.price, 'trade.price');
  if (input.tradeId !== undefined) {
    assertNonEmptyString(input.tradeId, 'trade.tradeId', marketDataError);
  }
  return Object.freeze({
    instrument: input.instrument,
    price: input.price,
    quantity: input.quantity,
    side: input.side,
    timestampMs: input.timestampMs,
    ...(input.tradeId === undefined ? {} : { tradeId: input.tradeId }),
  });
}

/**
 * 计算成交名义价值。
 *
 * @param trade - 成交
 * @returns quote 金额
 */
export function tradeNotional(trade: Trade): Money {
  return notionalMoney(trade.instrument, trade.price, trade.quantity);
}

/**
 * 创建 K 线。
 *
 * @param input - OHLCV、周期与时间区间
 * @returns 冻结 K 线
 * @throws {MarketDataValidationError} OHLC 关系、tick、周期或时间区间非法
 */
export function createBar(input: CreateBarInput): Bar {
  assertInstrumentValue(input.instrument, 'bar.instrument');
  assertOneOf(input.interval, BAR_INTERVALS, 'bar.interval', marketDataError);
  assertTimestampMs(input.openTimeMs, 'bar.openTimeMs', marketDataError);
  assertTimestampMs(input.closeTimeMs, 'bar.closeTimeMs', marketDataError);
  assertPositiveDecimal(input.open, 'bar.open', marketDataError);
  assertPositiveDecimal(input.high, 'bar.high', marketDataError);
  assertPositiveDecimal(input.low, 'bar.low', marketDataError);
  assertPositiveDecimal(input.close, 'bar.close', marketDataError);
  assertNonNegativeDecimal(input.volume, 'bar.volume', marketDataError);
  if (input.tradeCount !== undefined) {
    assertSafeInteger(input.tradeCount, 'bar.tradeCount', marketDataError);
  }
  assertMarketPrice(input.instrument, input.open, 'bar.open');
  assertMarketPrice(input.instrument, input.high, 'bar.high');
  assertMarketPrice(input.instrument, input.low, 'bar.low');
  assertMarketPrice(input.instrument, input.close, 'bar.close');

  const expectedMs = barIntervalMs(input.interval);
  if (input.closeTimeMs - input.openTimeMs !== expectedMs) {
    throw new MarketDataValidationError(
      `bar 时间区间必须是 ${expectedMs}ms（实际 ${input.closeTimeMs - input.openTimeMs}ms）`,
    );
  }
  if (input.high.compare(input.open) < 0 || input.high.compare(input.close) < 0) {
    throw new MarketDataValidationError('bar.high 必须不低于 open 与 close');
  }
  if (input.low.compare(input.open) > 0 || input.low.compare(input.close) > 0) {
    throw new MarketDataValidationError('bar.low 必须不高于 open 与 close');
  }
  if (input.high.compare(input.low) < 0) {
    throw new MarketDataValidationError('bar.high 不能低于 low');
  }
  return Object.freeze({
    instrument: input.instrument,
    interval: input.interval,
    openTimeMs: input.openTimeMs,
    closeTimeMs: input.closeTimeMs,
    open: input.open,
    high: input.high,
    low: input.low,
    close: input.close,
    volume: input.volume,
    ...(input.tradeCount === undefined ? {} : { tradeCount: input.tradeCount }),
  });
}

/**
 * K 线振幅。
 *
 * @param bar - K 线
 * @returns `high - low`
 */
export function barRange(bar: Bar): Decimal {
  return bar.high.subtract(bar.low);
}

/**
 * K 线典型价。
 *
 * @param bar - K 线
 * @returns `(high + low + close) / 3`，scale 18、half-even
 */
export function barTypicalPrice(bar: Bar): Decimal {
  return bar.high.add(bar.low).add(bar.close).divide(Decimal.fromInteger(3));
}

/**
 * K 线名义成交额。
 *
 * @param bar - K 线
 * @returns 典型价 * volume * contractMultiplier
 */
export function barNotional(bar: Bar): Money {
  return notionalMoney(bar.instrument, barTypicalPrice(bar), bar.volume);
}

/**
 * 判断 K 线是否收涨。
 *
 * @param bar - K 线
 * @returns close > open 时 true
 */
export function barIsBullish(bar: Bar): boolean {
  return bar.close.compare(bar.open) > 0;
}

/**
 * 创建 Ticker。
 *
 * @param input - 最新价、可选买卖价/标记价/24h 量与时间
 * @returns 冻结 Ticker
 * @throws {MarketDataValidationError} 字段非法或 bid >= ask
 */
export function createTicker(input: CreateTickerInput): Ticker {
  assertInstrumentValue(input.instrument, 'ticker.instrument');
  assertPositiveDecimal(input.lastPrice, 'ticker.lastPrice', marketDataError);
  assertTimestampMs(input.timestampMs, 'ticker.timestampMs', marketDataError);
  assertMarketPrice(input.instrument, input.lastPrice, 'ticker.lastPrice');

  const bid = input.bid;
  const ask = input.ask;
  if (bid !== undefined) {
    assertPositiveDecimal(bid, 'ticker.bid', marketDataError);
    assertMarketPrice(input.instrument, bid, 'ticker.bid');
  }
  if (ask !== undefined) {
    assertPositiveDecimal(ask, 'ticker.ask', marketDataError);
    assertMarketPrice(input.instrument, ask, 'ticker.ask');
  }
  if (bid !== undefined && ask !== undefined && bid.compare(ask) >= 0) {
    throw new MarketDataValidationError('ticker.bid 必须小于 ask');
  }
  if (input.markPrice !== undefined) {
    assertPositiveDecimal(input.markPrice, 'ticker.markPrice', marketDataError);
  }
  if (input.volume24h !== undefined) {
    assertNonNegativeDecimal(input.volume24h, 'ticker.volume24h', marketDataError);
  }

  return Object.freeze({
    instrument: input.instrument,
    lastPrice: input.lastPrice,
    ...(bid === undefined ? {} : { bid }),
    ...(ask === undefined ? {} : { ask }),
    ...(input.markPrice === undefined ? {} : { markPrice: input.markPrice }),
    ...(input.volume24h === undefined ? {} : { volume24h: input.volume24h }),
    timestampMs: input.timestampMs,
  });
}

/**
 * 创建盘口快照。
 *
 * @param input - 买卖盘、时间与序列号
 * @returns 冻结快照
 * @throws {MarketDataValidationError} 档位/排序/交叉盘口/序列非法
 */
export function createOrderBook(input: CreateOrderBookInput): OrderBookSnapshot {
  assertInstrumentValue(input.instrument, 'orderBook.instrument');
  assertTimestampMs(input.timestampMs, 'orderBook.timestampMs', marketDataError);
  assertSafeInteger(input.sequence, 'orderBook.sequence', marketDataError);
  if (!Array.isArray(input.bids) || !Array.isArray(input.asks)) {
    throw new MarketDataValidationError('orderBook.bids/asks 必须是数组');
  }
  const bids = normalizeBookLevels(input.instrument, input.bids, BookSide.Bids, false);
  const asks = normalizeBookLevels(input.instrument, input.asks, BookSide.Asks, false);
  assertNotCrossed(bids, asks);
  return Object.freeze({
    instrument: input.instrument,
    bids,
    asks,
    timestampMs: input.timestampMs,
    sequence: input.sequence,
  });
}

/**
 * 创建盘口增量。
 *
 * @param input - 增量档位、时间与序列号
 * @returns 冻结增量
 * @throws {MarketDataValidationError} 档位/数量/序列非法
 */
export function createOrderBookDelta(input: CreateOrderBookDeltaInput): OrderBookDelta {
  assertInstrumentValue(input.instrument, 'orderBookDelta.instrument');
  assertTimestampMs(input.timestampMs, 'orderBookDelta.timestampMs', marketDataError);
  assertSafeInteger(input.sequence, 'orderBookDelta.sequence', marketDataError);
  if (!Array.isArray(input.bids) || !Array.isArray(input.asks)) {
    throw new MarketDataValidationError('orderBookDelta.bids/asks 必须是数组');
  }
  const bids = normalizeDeltaLevels(input.instrument, input.bids, BookSide.Bids);
  const asks = normalizeDeltaLevels(input.instrument, input.asks, BookSide.Asks);
  return Object.freeze({
    instrument: input.instrument,
    bids,
    asks,
    timestampMs: input.timestampMs,
    sequence: input.sequence,
  });
}

/**
 * 应用盘口增量。
 *
 * @param snapshot - 当前快照
 * @param delta - 下一个序列的增量
 * @returns 新快照，不修改入参
 * @throws {MarketDataValidationError} 标的/序列不连续或应用后盘口非法
 */
export function applyOrderBookDelta(
  snapshot: OrderBookSnapshot,
  delta: OrderBookDelta,
): OrderBookSnapshot {
  if (!sameInstrument(snapshot.instrument, delta.instrument)) {
    throw new MarketDataValidationError(
      `盘口增量标的不一致（snapshot=${instrumentKey(snapshot.instrument)}, delta=${instrumentKey(delta.instrument)}）`,
    );
  }
  if (delta.sequence !== snapshot.sequence + 1) {
    throw new MarketDataValidationError(
      `盘口增量序列不连续（期望 ${snapshot.sequence + 1}，实际 ${delta.sequence}）`,
    );
  }

  const bids = sortBookLevels(mergeBookLevels(snapshot.bids, delta.bids), BookSide.Bids);
  const asks = sortBookLevels(mergeBookLevels(snapshot.asks, delta.asks), BookSide.Asks);
  return createOrderBook({
    instrument: snapshot.instrument,
    bids,
    asks,
    timestampMs: delta.timestampMs,
    sequence: delta.sequence,
  });
}

/**
 * 最优买价。
 *
 * @param book - 盘口
 * @returns 买一价；空买盘返回 undefined
 */
export function bookBestBid(book: OrderBookSnapshot): Decimal | undefined {
  return book.bids[0]?.price;
}

/**
 * 最优卖价。
 *
 * @param book - 盘口
 * @returns 卖一价；空卖盘返回 undefined
 */
export function bookBestAsk(book: OrderBookSnapshot): Decimal | undefined {
  return book.asks[0]?.price;
}

/**
 * 盘口中间价。
 *
 * @param book - 盘口
 * @returns 双边都存在时的中间价；否则 undefined
 */
export function bookMidPrice(book: OrderBookSnapshot): Decimal | undefined {
  const bid = bookBestBid(book);
  const ask = bookBestAsk(book);
  if (bid === undefined || ask === undefined) return undefined;
  return bid.add(ask).divide(Decimal.fromInteger(2));
}

/**
 * 盘口价差。
 *
 * @param book - 盘口
 * @returns 双边都存在时的 `ask - bid`；否则 undefined
 */
export function bookSpread(book: OrderBookSnapshot): Decimal | undefined {
  const bid = bookBestBid(book);
  const ask = bookBestAsk(book);
  if (bid === undefined || ask === undefined) return undefined;
  return ask.subtract(bid);
}

/**
 * 盘口价差（基点）。
 *
 * @param book - 盘口
 * @returns 双边都存在时的 `spread / mid * 10000`；否则 undefined
 */
export function bookSpreadBps(book: OrderBookSnapshot): Decimal | undefined {
  const mid = bookMidPrice(book);
  const spread = bookSpread(book);
  if (mid === undefined || spread === undefined || mid.isZero()) return undefined;
  return spread
    .multiply(Decimal.fromInteger(10_000))
    .divide(mid, { scale: 8, rounding: 'half-even' });
}

/**
 * 盘口一侧总数量。
 *
 * @param book - 盘口
 * @param side - 买盘或卖盘
 * @returns 数量之和
 */
export function bookDepth(book: OrderBookSnapshot, side: BookSide): Decimal {
  const levels = side === BookSide.Bids ? book.bids : book.asks;
  let total = Decimal.ZERO;
  for (const level of levels) total = total.add(level.quantity);
  return total;
}

/**
 * 盘口一侧名义价值。
 *
 * @param book - 盘口
 * @param side - 买盘或卖盘
 * @returns `Σ(price * quantity * multiplier)` 的 quote 金额
 */
export function bookDepthNotional(book: OrderBookSnapshot, side: BookSide): Money {
  const levels = side === BookSide.Bids ? book.bids : book.asks;
  if (levels.length === 0) return createMoney(Decimal.ZERO, book.instrument.quoteCurrency);
  const values = levels.map((level) => notionalMoney(book.instrument, level.price, level.quantity));
  return sumMoney(values, book.instrument.quoteCurrency);
}

/**
 * 盘口失衡度。
 *
 * @param book - 盘口
 * @returns `(bidDepth - askDepth) / (bidDepth + askDepth)`，scale 8；空盘口返回 0
 */
export function bookImbalance(book: OrderBookSnapshot): Decimal {
  const bidDepth = bookDepth(book, BookSide.Bids);
  const askDepth = bookDepth(book, BookSide.Asks);
  const total = bidDepth.add(askDepth);
  if (total.isZero()) return Decimal.ZERO;
  return bidDepth.subtract(askDepth).divide(total, { scale: 8, rounding: 'half-even' });
}

/**
 * 盘口档位名义价值总额（便捷函数）。
 *
 * @param book - 盘口
 * @returns 买卖两侧名义价值之和
 */
export function bookTotalNotional(book: OrderBookSnapshot): Money {
  return sumMoney(
    [bookDepthNotional(book, BookSide.Bids), bookDepthNotional(book, BookSide.Asks)],
    book.instrument.quoteCurrency,
  );
}

/** 校验标的。 */
function assertInstrumentValue(value: Instrument, path: string): void {
  if (!isInstrument(value)) {
    throw new MarketDataValidationError(`${path} 非法`);
  }
}

/** 校验行情价格必须是 tickSize 的正整数倍。 */
function assertMarketPrice(instrument: Instrument, price: Decimal, path: string): void {
  if (!isPriceValid(instrument, price)) {
    throw new MarketDataValidationError(
      `${path} 必须是 tickSize ${instrument.tickSize.toString()} 的正整数倍（实际值: ${price.toString()}）`,
    );
  }
}

/** 按盘口侧排序合并后的档位。 */
function sortBookLevels(
  levels: readonly OrderBookLevel[],
  side: BookSide,
): readonly OrderBookLevel[] {
  return Object.freeze(
    [...levels].sort((left, right) =>
      side === BookSide.Bids ? right.price.compare(left.price) : left.price.compare(right.price),
    ),
  );
}

/** 校验并冻结有序盘口档位。 */
function normalizeBookLevels(
  instrument: Instrument,
  levels: readonly OrderBookLevel[],
  side: BookSide,
  allowZeroQuantity: boolean,
): readonly OrderBookLevel[] {
  assertArrayMaxLength(levels, `orderBook.${side}`, MAX_ORDER_BOOK_LEVELS, marketDataError);
  const seen = new Set<string>();
  const normalized: OrderBookLevel[] = [];

  for (let index = 0; index < levels.length; index += 1) {
    const level = levels[index];
    if (level === undefined || !isPlainObject(level)) {
      throw new MarketDataValidationError(`orderBook.${side}[${index}] 必须是对象`);
    }
    const price = level['price'];
    const quantity = level['quantity'];
    assertPositiveDecimal(price, `orderBook.${side}[${index}].price`, marketDataError);
    if (allowZeroQuantity) {
      assertNonNegativeDecimal(quantity, `orderBook.${side}[${index}].quantity`, marketDataError);
    } else {
      assertPositiveDecimal(quantity, `orderBook.${side}[${index}].quantity`, marketDataError);
    }
    assertMarketPrice(instrument, price, `orderBook.${side}[${index}].price`);

    const key = price.toString();
    if (seen.has(key)) {
      throw new MarketDataValidationError(`orderBook.${side} 价格重复: ${key}`);
    }
    seen.add(key);

    const previous = normalized[normalized.length - 1];
    if (previous !== undefined) {
      const comparison = previous.price.compare(price);
      const isSorted = side === BookSide.Bids ? comparison > 0 : comparison < 0;
      if (!isSorted) {
        throw new MarketDataValidationError(
          `orderBook.${side} 必须${side === BookSide.Bids ? '严格递减' : '严格递增'}`,
        );
      }
    }

    normalized.push(Object.freeze({ price, quantity }));
  }

  return Object.freeze(normalized);
}

/** 校验增量档位：允许任意顺序与零数量删除。 */
function normalizeDeltaLevels(
  instrument: Instrument,
  levels: readonly OrderBookLevel[],
  side: BookSide,
): readonly OrderBookLevel[] {
  assertArrayMaxLength(levels, `orderBookDelta.${side}`, MAX_ORDER_BOOK_LEVELS, marketDataError);
  const seen = new Set<string>();
  const normalized: OrderBookLevel[] = [];
  for (let index = 0; index < levels.length; index += 1) {
    const level = levels[index];
    if (level === undefined || !isPlainObject(level)) {
      throw new MarketDataValidationError(`orderBookDelta.${side}[${index}] 必须是对象`);
    }
    const price = level['price'];
    const quantity = level['quantity'];
    assertPositiveDecimal(price, `orderBookDelta.${side}[${index}].price`, marketDataError);
    assertNonNegativeDecimal(
      quantity,
      `orderBookDelta.${side}[${index}].quantity`,
      marketDataError,
    );
    assertMarketPrice(instrument, price, `orderBookDelta.${side}[${index}].price`);
    const key = price.toString();
    if (seen.has(key)) {
      throw new MarketDataValidationError(`orderBookDelta.${side} 价格重复: ${key}`);
    }
    seen.add(key);
    normalized.push(Object.freeze({ price, quantity }));
  }
  return Object.freeze(normalized);
}

/** 合并盘口档位。 */
function mergeBookLevels(
  current: readonly OrderBookLevel[],
  updates: readonly OrderBookLevel[],
): readonly OrderBookLevel[] {
  const map = new Map<string, OrderBookLevel>();
  for (const level of current) map.set(level.price.toString(), level);
  for (const level of updates) {
    const key = level.price.toString();
    if (level.quantity.isZero()) {
      map.delete(key);
    } else {
      map.set(key, Object.freeze({ price: level.price, quantity: level.quantity }));
    }
  }
  return Object.freeze([...map.values()]);
}

/** 校验交叉盘口。 */
function assertNotCrossed(bids: readonly OrderBookLevel[], asks: readonly OrderBookLevel[]): void {
  const bestBid = bids[0]?.price;
  const bestAsk = asks[0]?.price;
  if (bestBid !== undefined && bestAsk !== undefined && bestBid.compare(bestAsk) >= 0) {
    throw new MarketDataValidationError(
      `交叉盘口：best bid ${bestBid.toString()} >= best ask ${bestAsk.toString()}`,
    );
  }
}
