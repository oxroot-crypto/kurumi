/**
 * @file Binance 现货/期货提供方测试。
 * @description 用固定公开 API JSON 覆盖行情、K 线、成交与参考数据映射。
 * @module binance/provider.test
 */

import { describe, expect, it } from 'vitest';

import {
  AssetClass,
  BarInterval,
  ProviderHttpError,
  createCurrency,
  createInstrument,
  instrumentKey,
  type FetchLike,
  type Instrument,
} from '@kurumi/finance';
import { Decimal } from '@kurumi/math';

import {
  createBinanceFuturesMarketDataProvider,
  createBinanceFuturesReferenceDataProvider,
  createBinanceMarketDataProvider,
  createBinanceReferenceDataProvider,
} from './index.js';

const T0 = Date.UTC(2026, 9, 19);
const USDT = createCurrency('USDT', 8);
const btcUsdt = createInstrument({
  assetClass: AssetClass.Crypto,
  venue: 'binance',
  symbol: 'BTC/USDT',
  baseAsset: 'BTC',
  quoteCurrency: USDT,
  tickSize: Decimal.parse('0.01'),
  lotSize: Decimal.parse('0.001'),
}) as Instrument;
const btcPerp = createInstrument({
  assetClass: AssetClass.Perpetual,
  venue: 'binance-futures',
  symbol: 'BTCUSDT',
  quoteCurrency: USDT,
  tickSize: Decimal.parse('0.1'),
  lotSize: Decimal.parse('0.001'),
  fundingIntervalMs: 28_800_000,
}) as Instrument;

function fetchFor(routes: Readonly<Record<string, unknown>>): FetchLike {
  return async (input) => {
    const path = new URL(input).pathname;
    const payload = routes[path];
    if (payload === undefined) {
      return new Response('{"message":"not found"}', { status: 404 });
    }
    return new Response(JSON.stringify(payload), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    });
  };
}

const spotRoutes = {
  '/api/v3/ticker/bookTicker': {
    bidPrice: '81596.25',
    bidQty: '2.9',
    askPrice: '81596.26',
    askQty: '4.1',
  },
  '/api/v3/ticker/24hr': {
    lastPrice: '81596.25',
    bidPrice: '81596.25',
    askPrice: '81596.26',
    volume: '123',
    closeTime: T0,
  },
  '/api/v3/depth': {
    lastUpdateId: 5,
    bids: [['81596.25', '1']],
    asks: [['81596.26', '2']],
  },
  '/api/v3/klines': [[T0, '100', '110', '90', '105', '10', T0 + 59_999, '0', 3]],
  '/api/v3/aggTrades': [{ a: 1, p: '100', q: '1', T: T0, m: false }],
};

describe('Binance spot market data', () => {
  it('解析 quote/ticker/order book/bars/trades', async () => {
    const provider = createBinanceMarketDataProvider({
      fetchImpl: fetchFor(spotRoutes),
      nowMs: () => T0,
    });
    const quote = await provider.getQuote?.(btcUsdt);
    expect(quote?.bid.toString()).toBe('81596.25');
    expect(quote?.ask.toString()).toBe('81596.26');

    const ticker = await provider.getTicker?.(btcUsdt);
    expect(ticker?.lastPrice.toString()).toBe('81596.25');
    expect(ticker?.volume24h?.toString()).toBe('123');

    const book = await provider.getOrderBook?.(btcUsdt, { depth: 10 });
    expect(book?.sequence).toBe(5);
    expect(book?.bids[0]?.price.toString()).toBe('81596.25');

    const bars = await provider.getBars?.(btcUsdt, {
      interval: BarInterval.Minute1,
      fromMs: T0,
      toMs: T0 + 60_000,
    });
    expect(bars?.[0]?.close.toString()).toBe('105');
    expect(bars?.[0]?.closeTimeMs).toBe(T0 + 60_000);
    expect(bars?.[0]?.tradeCount).toBe(3);

    const trades = await provider.getTrades?.(btcUsdt, { limit: 1 });
    expect(trades?.[0]?.side).toBe('buy');
    expect(trades?.[0]?.tradeId).toBe('1');
  });

  it('拒绝非法 depth/limit', async () => {
    const provider = createBinanceMarketDataProvider({ fetchImpl: fetchFor(spotRoutes) });
    await expect(provider.getOrderBook?.(btcUsdt, { depth: 0 })).rejects.toThrow();
    await expect(provider.getTrades?.(btcUsdt, { limit: 0 })).rejects.toThrow();
  });
});

const exchangeInfo = {
  symbols: [
    {
      symbol: 'BTCUSDT',
      status: 'TRADING',
      baseAsset: 'BTC',
      quoteAsset: 'USDT',
      filters: [
        { filterType: 'PRICE_FILTER', tickSize: '0.01' },
        { filterType: 'LOT_SIZE', stepSize: '0.001', minQty: '0.001', maxQty: '1000' },
        { filterType: 'NOTIONAL', minNotional: '10' },
      ],
    },
    {
      symbol: 'ETHUSDT',
      status: 'BREAK',
      baseAsset: 'ETH',
      quoteAsset: 'USDT',
      filters: [
        { filterType: 'PRICE_FILTER', tickSize: '0.01' },
        { filterType: 'LOT_SIZE', stepSize: '0.001', minQty: '0.001', maxQty: '1000' },
      ],
    },
  ],
};

describe('Binance spot reference data', () => {
  it('映射标的、过滤非交易状态与币种', async () => {
    const provider = createBinanceReferenceDataProvider({
      fetchImpl: fetchFor({ '/api/v3/exchangeInfo': exchangeInfo }),
      quoteAssets: ['USDT'],
    });
    const instruments = await provider.listInstruments();
    expect(instruments).toHaveLength(1);
    expect(instrumentKey(instruments[0]!)).toBe('binance:BTC/USDT');
    expect(instruments[0]?.assetClass).toBe('crypto');
    expect(instruments[0]?.minNotional?.amount.toString()).toBe('10');

    expect((await provider.getInstrument('binance:BTC/USDT'))?.symbol).toBe('BTC/USDT');
    expect(await provider.getInstrument('binance:ETH/USDT')).toBeUndefined();
    expect((await provider.listCurrencies()).map((item) => item.code)).toEqual(['USDT']);

    const all = createBinanceReferenceDataProvider({
      fetchImpl: fetchFor({ '/api/v3/exchangeInfo': exchangeInfo }),
      includeNonTrading: true,
    });
    expect(await all.listInstruments()).toHaveLength(2);
  });

  it('参考数据查询传递取消信号', async () => {
    const controller = new AbortController();
    controller.abort();
    let observed = false;
    const provider = createBinanceReferenceDataProvider({
      fetchImpl: async (_input, init) => {
        observed = init?.signal?.aborted === true;
        throw new Error('aborted');
      },
    });
    await expect(provider.listInstruments({ signal: controller.signal })).rejects.toBeInstanceOf(
      ProviderHttpError,
    );
    expect(observed).toBe(true);
  });
});

const futuresExchangeInfo = {
  symbols: [
    {
      symbol: 'BTCUSDT',
      status: 'TRADING',
      contractType: 'PERPETUAL',
      baseAsset: 'BTC',
      quoteAsset: 'USDT',
      fundingIntervalHours: 8,
      filters: [
        { filterType: 'PRICE_FILTER', tickSize: '0.1' },
        { filterType: 'LOT_SIZE', stepSize: '0.001', minQty: '0.001' },
        { filterType: 'MIN_NOTIONAL', notional: '5' },
      ],
    },
    {
      symbol: 'BTCUSDT_260116',
      status: 'TRADING',
      contractType: 'CURRENT_QUARTER',
      baseAsset: 'BTC',
      quoteAsset: 'USDT',
      deliveryDate: T0 + 90 * 86_400_000,
      filters: [
        { filterType: 'PRICE_FILTER', tickSize: '0.1' },
        { filterType: 'LOT_SIZE', stepSize: '0.001', minQty: '0.001' },
      ],
    },
  ],
};

describe('Binance futures reference data', () => {
  it('映射永续与季度期货', async () => {
    const provider = createBinanceFuturesReferenceDataProvider({
      fetchImpl: fetchFor({ '/fapi/v1/exchangeInfo': futuresExchangeInfo }),
      quoteAssets: ['USDT'],
    });
    const instruments = await provider.listInstruments();
    expect(instruments).toHaveLength(2);
    const perpetual = instruments.find((item) => item.assetClass === 'perpetual');
    const future = instruments.find((item) => item.assetClass === 'future');
    expect(perpetual?.symbol).toBe('BTCUSDT');
    expect(perpetual?.fundingIntervalMs).toBe(28_800_000);
    expect(future?.symbol).toBe('BTCUSDT_260116');
    expect(future?.assetClass === 'future' && future.expiryMs).toBe(T0 + 90 * 86_400_000);

    const perpetualOnly = createBinanceFuturesReferenceDataProvider({
      fetchImpl: fetchFor({ '/fapi/v1/exchangeInfo': futuresExchangeInfo }),
      contractTypes: ['PERPETUAL'],
    });
    const filtered = await perpetualOnly.listInstruments();
    expect(filtered).toHaveLength(1);
    expect(filtered[0]?.assetClass).toBe('perpetual');
    expect(await perpetualOnly.getInstrument('binance-futures:NOPE')).toBeUndefined();
  });
});

describe('Binance futures market data', () => {
  it('解析 quote/ticker/order book/bars/trades', async () => {
    const provider = createBinanceFuturesMarketDataProvider({
      fetchImpl: fetchFor({
        '/fapi/v1/ticker/bookTicker': {
          bidPrice: '81596.2',
          bidQty: '1',
          askPrice: '81596.3',
          askQty: '2',
        },
        '/fapi/v1/ticker/24hr': {
          lastPrice: '81596.2',
          bidPrice: '81596.2',
          askPrice: '81596.3',
          volume: '99',
          closeTime: T0,
        },
        '/fapi/v1/depth': {
          lastUpdateId: 7,
          bids: [['81596.2', '1']],
          asks: [['81596.3', '2']],
        },
        '/fapi/v1/klines': [[T0, '100', '110', '90', '105', '10', T0 + 59_999, '0', 3]],
        '/fapi/v1/aggTrades': [{ a: 2, p: '100', q: '1', T: T0, m: true }],
      }),
      nowMs: () => T0,
    });
    const quote = await provider.getQuote?.(btcPerp);
    expect(quote?.bid.toString()).toBe('81596.2');
    const ticker = await provider.getTicker?.(btcPerp);
    expect(ticker?.volume24h?.toString()).toBe('99');
    const book = await provider.getOrderBook?.(btcPerp, { depth: 10 });
    expect(book?.sequence).toBe(7);
    const bars = await provider.getBars?.(btcPerp, {
      interval: BarInterval.Minute1,
      fromMs: T0,
      toMs: T0 + 60_000,
    });
    expect(bars?.[0]?.close.toString()).toBe('105');
    const trades = await provider.getTrades?.(btcPerp, { limit: 1 });
    expect(trades?.[0]?.side).toBe('sell');
  });

  it('拒绝非法 depth/limit', async () => {
    const provider = createBinanceFuturesMarketDataProvider({ fetchImpl: fetchFor({}) });
    await expect(provider.getOrderBook?.(btcPerp, { depth: 0 })).rejects.toThrow();
    await expect(provider.getTrades?.(btcPerp, { limit: 0 })).rejects.toThrow();
  });
});
