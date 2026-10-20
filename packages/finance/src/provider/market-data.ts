/**
 * @file 只读行情提供方契约。
 * @description 只定义报价、盘口、K 线与成交查询；不包含下单、撤单或任何写操作。
 * @module finance/provider/market-data
 */

import type { Instrument } from '../instrument.js';
import type { Bar, BarInterval, OrderBookSnapshot, Quote, Ticker, Trade } from '../market-data.js';

/** 获取 K 线选项。 */
export interface GetBarsOptions {
  /** K 线周期。 */
  readonly interval: BarInterval;
  /** 起始时间（epoch 毫秒，含）。 */
  readonly fromMs: number;
  /** 结束时间（epoch 毫秒，不含）。 */
  readonly toMs: number;
  /** 取消信号。 */
  readonly signal?: AbortSignal;
}

/** 获取盘口选项。 */
export interface GetOrderBookOptions {
  /** 最大档位；缺省由提供方决定。 */
  readonly depth?: number;
  /** 取消信号。 */
  readonly signal?: AbortSignal;
}

/** 获取成交选项。 */
export interface GetTradesOptions {
  /** 最大条数。 */
  readonly limit?: number;
  /** 取消信号。 */
  readonly signal?: AbortSignal;
}

/** 只读行情提供方。 */
export interface MarketDataProvider {
  /** 提供方 id，例如 binance、polygon。 */
  readonly id: string;
  /**
   * 获取最新报价。
   *
   * @param instrument - 标的
   * @param signal - 取消信号
   */
  getQuote?(instrument: Instrument, signal?: AbortSignal): Promise<Quote>;
  /**
   * 获取 Ticker 快照（可选能力）。
   *
   * @param instrument - 标的
   * @param signal - 取消信号
   */
  getTicker?(instrument: Instrument, signal?: AbortSignal): Promise<Ticker>;
  /**
   * 获取盘口快照。
   *
   * @param instrument - 标的
   * @param options - 档位与取消信号
   */
  getOrderBook?(instrument: Instrument, options?: GetOrderBookOptions): Promise<OrderBookSnapshot>;
  /**
   * 获取 K 线。
   *
   * @param instrument - 标的
   * @param options - 周期、时间范围与取消信号
   */
  getBars?(instrument: Instrument, options: GetBarsOptions): Promise<readonly Bar[]>;
  /**
   * 获取最近成交（可选能力）。
   *
   * @param instrument - 标的
   * @param options - 条数与取消信号
   */
  getTrades?(instrument: Instrument, options?: GetTradesOptions): Promise<readonly Trade[]>;
}
