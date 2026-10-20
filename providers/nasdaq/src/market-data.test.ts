/**
 * @file Nasdaq 行情提供方测试。
 * @description 用固定 JSON 响应覆盖 Ticker、日线、错误与不支持能力。
 * @module nasdaq/market-data.test
 */

import { describe, expect, it } from 'vitest';

import {
  BarInterval,
  ProviderHttpError,
  UnsupportedProviderOperationError,
  createCurrency,
  createInstrument,
  type FetchLike,
  type SpotInstrument,
} from '@kurumi/finance';
import { Decimal } from '@kurumi/math';

import { createNasdaqMarketDataProvider } from './market-data.js';

const T0 = Date.UTC(2026, 9, 19);
const USD = createCurrency('USD', 2);
const aapl = createInstrument({
  assetClass: 'equity',
  venue: 'nasdaq',
  symbol: 'AAPL',
  baseAsset: 'AAPL',
  quoteCurrency: USD,
  tickSize: Decimal.parse('0.01'),
  lotSize: Decimal.ONE,
}) as SpotInstrument;

function jsonFetch(payload: unknown, status = 200): FetchLike {
  return async () =>
    new Response(JSON.stringify(payload), {
      status,
      headers: { 'content-type': 'application/json' },
    });
}

describe('createNasdaqMarketDataProvider', () => {
  it('解析 Ticker', async () => {
    const provider = createNasdaqMarketDataProvider({
      fetchImpl: jsonFetch({
        data: {
          primaryData: {
            lastSalePrice: '$342.94',
            bidPrice: '$342.90',
            askPrice: '$342.98',
            bidSize: '100',
            askSize: '200',
            volume: '34,573,310',
            lastTradeTimestamp: 'Oct 19, 2026',
          },
        },
      }),
      nowMs: () => T0,
    });
    const ticker = await provider.getTicker?.(aapl);
    expect(ticker?.lastPrice.toString()).toBe('342.94');
    expect(ticker?.bid?.toString()).toBe('342.9');
    expect(ticker?.ask?.toString()).toBe('342.98');
    expect(ticker?.volume24h?.toString()).toBe('34573310');
    expect(ticker?.timestampMs).toBe(T0);

    const quote = await provider.getQuote?.(aapl);
    expect(quote?.bid.toString()).toBe('342.9');
    expect(quote?.ask.toString()).toBe('342.98');
    expect(quote?.bidSize.toString()).toBe('100');
    expect(quote?.askSize.toString()).toBe('200');
    expect(provider.getOrderBook).toBeUndefined();
  });

  it('按 tickSize 对齐 Ticker/Quote 中的子最小变动价位', async () => {
    const provider = createNasdaqMarketDataProvider({
      fetchImpl: jsonFetch({
        data: {
          primaryData: {
            lastSalePrice: '$765.4525',
            bidPrice: '$765.399',
            askPrice: '$765.464',
            bidSize: '400',
            askSize: '440',
            lastTradeTimestamp: 'Oct 20, 2026',
          },
        },
      }),
      nowMs: () => T0,
    });
    const ticker = await provider.getTicker?.(aapl);
    expect(ticker?.lastPrice.toString()).toBe('765.45');
    const quote = await provider.getQuote?.(aapl);
    expect(quote?.bid.toString()).toBe('765.4');
    expect(quote?.ask.toString()).toBe('765.46');
  });

  it('解析日线并向上/向下对齐 tick', async () => {
    const provider = createNasdaqMarketDataProvider({
      fetchImpl: jsonFetch({
        data: {
          tradesTable: {
            rows: [
              {
                date: '10/19/2026',
                close: '$342.94',
                volume: '34,573,180',
                open: '$340.775',
                high: '$346.00',
                low: '$340.115',
              },
            ],
          },
        },
      }),
      nowMs: () => T0,
    });
    const bars = await provider.getBars?.(aapl, {
      interval: BarInterval.Day1,
      fromMs: Date.UTC(2026, 9, 1),
      toMs: Date.UTC(2026, 9, 20),
    });
    expect(bars).toHaveLength(1);
    expect(bars?.[0]?.open.toString()).toBe('340.78');
    expect(bars?.[0]?.low.toString()).toBe('340.12');
    expect(bars?.[0]?.close.toString()).toBe('342.94');
    expect(bars?.[0]?.closeTimeMs).toBe(Date.UTC(2026, 9, 20));
  });

  it('拒绝不支持周期与非法时间范围', async () => {
    const provider = createNasdaqMarketDataProvider({
      fetchImpl: jsonFetch({ data: {} }),
      nowMs: () => T0,
    });
    await expect(
      provider.getBars?.(aapl, {
        interval: BarInterval.Minute1,
        fromMs: T0,
        toMs: T0 + 1,
      }),
    ).rejects.toBeInstanceOf(UnsupportedProviderOperationError);
    await expect(
      provider.getBars?.(aapl, {
        interval: BarInterval.Day1,
        fromMs: T0,
        toMs: T0,
      }),
    ).rejects.toThrow();
  });

  it('HTTP 错误包装为 ProviderHttpError', async () => {
    const provider = createNasdaqMarketDataProvider({
      fetchImpl: jsonFetch({ message: 'boom' }, 500),
      nowMs: () => T0,
    });
    await expect(provider.getTicker?.(aapl)).rejects.toBeInstanceOf(ProviderHttpError);
  });

  it('处理空历史表与跳过缺失 OHLC 的行', async () => {
    const empty = createNasdaqMarketDataProvider({
      fetchImpl: jsonFetch({ data: { tradesTable: null } }),
      nowMs: () => T0,
    });
    await expect(
      empty.getBars?.(aapl, {
        interval: BarInterval.Day1,
        fromMs: Date.UTC(2026, 9, 1),
        toMs: Date.UTC(2026, 9, 20),
      }),
    ).resolves.toEqual([]);

    const partial = createNasdaqMarketDataProvider({
      fetchImpl: jsonFetch({
        data: {
          tradesTable: {
            rows: [
              { date: '10/19/2026', open: '$1', high: '$2', low: '$0.5', close: null },
              {
                date: '10/18/2026',
                open: '$1',
                high: '$2',
                low: '$0.5',
                close: '$1.5',
                volume: '',
              },
            ],
          },
        },
      }),
      nowMs: () => T0,
    });
    const bars = await partial.getBars?.(aapl, {
      interval: BarInterval.Day1,
      fromMs: Date.UTC(2026, 9, 1),
      toMs: Date.UTC(2026, 9, 20),
    });
    expect(bars).toHaveLength(1);
    expect(bars?.[0]?.volume.isZero()).toBe(true);
  });

  it('缺失必需价格时报数据错误', async () => {
    const provider = createNasdaqMarketDataProvider({
      fetchImpl: jsonFetch({ data: { primaryData: {} } }),
      nowMs: () => T0,
    });
    await expect(provider.getTicker?.(aapl)).rejects.toThrow();
  });
});
