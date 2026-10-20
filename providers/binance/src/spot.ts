/**
 * @file Binance 现货只读行情提供方。
 * @description 使用 api.binance.com 公共 REST：报价、Ticker、盘口、K 线与聚合成交。
 * @module binance/spot
 */

import {
  ProviderDataError,
  TradeSide,
  createOrderBook,
  createQuote,
  createTicker,
  createTrade,
  type Bar,
  type GetBarsOptions,
  type GetOrderBookOptions,
  type GetTradesOptions,
  type Instrument,
  type MarketDataProvider,
  type OrderBookLevel,
  type OrderBookSnapshot,
  type Quote,
  type Ticker,
  type Trade,
} from '@kurumi/finance';

import {
  buildBarFromKline,
  getBinanceJson,
  requireArray,
  requireDecimal,
  requireInteger,
  requireRecord,
  resolveBinanceOptions,
  toBinanceSymbol,
  type BinanceProviderOptions,
  type ResolvedBinanceOptions,
} from './support.js';

/**
 * 创建 Binance 现货行情提供方。
 *
 * @param options - API 根地址、fetch、货币精度与超时
 * @returns 只读 `MarketDataProvider`
 *
 * @example
 * ```ts
 * const provider = createBinanceMarketDataProvider();
 * const quote = await provider.getQuote!(btcUsdt);
 * ```
 */
export function createBinanceMarketDataProvider(
  options: BinanceProviderOptions = {},
): MarketDataProvider {
  const settings = resolveBinanceOptions(options);
  return Object.freeze({
    id: settings.id,
    getQuote: (instrument: Instrument, signal?: AbortSignal) =>
      getQuote(settings, instrument, signal),
    getTicker: (instrument: Instrument, signal?: AbortSignal) =>
      getTicker(settings, instrument, signal),
    getOrderBook: (instrument: Instrument, bookOptions?: GetOrderBookOptions) =>
      getOrderBook(settings, instrument, bookOptions),
    getBars: (instrument: Instrument, barsOptions: GetBarsOptions) =>
      getBars(settings, instrument, barsOptions),
    getTrades: (instrument: Instrument, tradesOptions?: GetTradesOptions) =>
      getTrades(settings, instrument, tradesOptions),
  });
}

/** 获取买一/卖一。 */
async function getQuote(
  settings: ResolvedBinanceOptions,
  instrument: Instrument,
  signal?: AbortSignal,
): Promise<Quote> {
  const payload = await getBinanceJson(
    settings,
    settings.baseUrl,
    '/api/v3/ticker/bookTicker',
    { symbol: toBinanceSymbol(instrument) },
    signal,
  );
  const record = requireRecord(payload, '$', settings.id);
  return createQuote({
    instrument,
    bid: requireDecimal(record['bidPrice'], 'bidPrice', settings.id),
    ask: requireDecimal(record['askPrice'], 'askPrice', settings.id),
    bidSize: requireDecimal(record['bidQty'], 'bidQty', settings.id),
    askSize: requireDecimal(record['askQty'], 'askQty', settings.id),
    timestampMs: settings.nowMs(),
  });
}

/** 获取 24h Ticker。 */
async function getTicker(
  settings: ResolvedBinanceOptions,
  instrument: Instrument,
  signal?: AbortSignal,
): Promise<Ticker> {
  const payload = await getBinanceJson(
    settings,
    settings.baseUrl,
    '/api/v3/ticker/24hr',
    { symbol: toBinanceSymbol(instrument) },
    signal,
  );
  const record = requireRecord(payload, '$', settings.id);
  const bid = requireDecimal(record['bidPrice'], 'bidPrice', settings.id);
  const ask = requireDecimal(record['askPrice'], 'askPrice', settings.id);
  return createTicker({
    instrument,
    lastPrice: requireDecimal(record['lastPrice'], 'lastPrice', settings.id),
    bid,
    ask,
    volume24h: requireDecimal(record['volume'], 'volume', settings.id),
    timestampMs: requireInteger(record['closeTime'], 'closeTime', settings.id),
  });
}

/** 获取盘口。 */
async function getOrderBook(
  settings: ResolvedBinanceOptions,
  instrument: Instrument,
  options?: GetOrderBookOptions,
): Promise<OrderBookSnapshot> {
  const limit = options?.depth ?? 100;
  if (!Number.isSafeInteger(limit) || limit <= 0 || limit > 5_000) {
    throw new ProviderDataError(settings.id, 'depth 必须是 1..5000 的安全整数');
  }
  const payload = await getBinanceJson(
    settings,
    settings.baseUrl,
    '/api/v3/depth',
    { symbol: toBinanceSymbol(instrument), limit },
    options?.signal,
  );
  const record = requireRecord(payload, '$', settings.id);
  return createOrderBook({
    instrument,
    bids: toLevels(record['bids'], 'bids', settings.id),
    asks: toLevels(record['asks'], 'asks', settings.id),
    sequence: requireInteger(record['lastUpdateId'], 'lastUpdateId', settings.id),
    timestampMs: settings.nowMs(),
  });
}

/** 获取 K 线。 */
async function getBars(
  settings: ResolvedBinanceOptions,
  instrument: Instrument,
  options: GetBarsOptions,
): Promise<readonly Bar[]> {
  const payload = await getBinanceJson(
    settings,
    settings.baseUrl,
    '/api/v3/klines',
    {
      symbol: toBinanceSymbol(instrument),
      interval: options.interval,
      limit: 1_000,
      startTime: options.fromMs,
      endTime: options.toMs,
    },
    options.signal,
  );
  const rows = requireArray(payload, '$', settings.id);
  const bars = rows.map((row, index) =>
    buildBarFromKline(
      settings,
      instrument,
      options.interval,
      requireArray(row, `$[${index}]`, settings.id),
    ),
  );
  return Object.freeze(
    bars.filter((bar) => bar.openTimeMs >= options.fromMs && bar.openTimeMs < options.toMs),
  );
}

/** 获取聚合成交。 */
async function getTrades(
  settings: ResolvedBinanceOptions,
  instrument: Instrument,
  options?: GetTradesOptions,
): Promise<readonly Trade[]> {
  const limit = options?.limit ?? 100;
  if (!Number.isSafeInteger(limit) || limit <= 0 || limit > 1_000) {
    throw new ProviderDataError(settings.id, 'limit 必须是 1..1000 的安全整数');
  }
  const payload = await getBinanceJson(
    settings,
    settings.baseUrl,
    '/api/v3/aggTrades',
    { symbol: toBinanceSymbol(instrument), limit },
    options?.signal,
  );
  const rows = requireArray(payload, '$', settings.id);
  return Object.freeze(
    rows.map((row, index) => {
      const record = requireRecord(row, `$[${index}]`, settings.id);
      return createTrade({
        instrument,
        price: requireDecimal(record['p'], 'p', settings.id),
        quantity: requireDecimal(record['q'], 'q', settings.id),
        side: record['m'] === true ? TradeSide.Sell : TradeSide.Buy,
        timestampMs: requireInteger(record['T'], 'T', settings.id),
        tradeId: String(requireInteger(record['a'], 'a', settings.id)),
      });
    }),
  );
}

/** 转换盘口档位。 */
function toLevels(value: unknown, path: string, providerId: string): readonly OrderBookLevel[] {
  const rows = requireArray(value, path, providerId);
  return Object.freeze(
    rows.map((row, index) => {
      const level = requireArray(row, `${path}[${index}]`, providerId);
      return Object.freeze({
        price: requireDecimal(level[0], `${path}[${index}][0]`, providerId),
        quantity: requireDecimal(level[1], `${path}[${index}][1]`, providerId),
      });
    }),
  );
}
