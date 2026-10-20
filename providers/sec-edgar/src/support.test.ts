/**
 * @file SEC EDGAR 共享解析与选项测试。
 * @description 覆盖 User-Agent、ticker 列表、标的映射与 CIK 解析边界。
 * @module sec-edgar/support.test
 */

import { describe, expect, it } from 'vitest';

import {
  ProviderDataError,
  createCurrency,
  createInstrument,
  type FetchLike,
  type Instrument,
} from '@kurumi/finance';
import { Decimal } from '@kurumi/math';

import {
  fetchTickers,
  findTicker,
  instrumentForTicker,
  resolveCik,
  resolveSecOptions,
} from './support.js';

const USD = createCurrency('USD', 2);
const aapl = createInstrument({
  assetClass: 'equity',
  venue: 'sec',
  symbol: 'AAPL',
  baseAsset: 'AAPL',
  quoteCurrency: USD,
  tickSize: Decimal.parse('0.01'),
  lotSize: Decimal.ONE,
}) as Instrument;
const userAgent = 'kurumi-test/0.1 test@example.com';

function jsonFetch(payload: unknown): FetchLike {
  return async () =>
    new Response(JSON.stringify(payload), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    });
}

describe('SEC EDGAR support', () => {
  it('解析 ticker 列表并跳过非法项', async () => {
    const settings = resolveSecOptions({
      userAgent,
      fetchImpl: jsonFetch({
        '0': { cik_str: 320193, ticker: 'aapl', title: 'Apple Inc.' },
        '1': { cik_str: 1, ticker: 'MSFT' },
        '2': { nope: true },
      }),
    });
    const tickers = await fetchTickers(settings);
    expect(tickers).toHaveLength(1);
    expect(tickers[0]?.ticker).toBe('AAPL');
    expect(tickers[0]?.cik).toBe('0000320193');
    expect(findTicker(tickers, aapl)?.title).toBe('Apple Inc.');
    const instrument = instrumentForTicker(settings, tickers[0]!);
    expect(instrument.symbol).toBe('AAPL');
  });

  it('拒绝非法 ticker 响应', async () => {
    const settings = resolveSecOptions({
      userAgent,
      fetchImpl: jsonFetch([]),
    });
    await expect(fetchTickers(settings)).rejects.toBeInstanceOf(ProviderDataError);
    const invalidEntry = resolveSecOptions({
      userAgent,
      fetchImpl: jsonFetch({ '0': 1 }),
    });
    await expect(fetchTickers(invalidEntry)).rejects.toBeInstanceOf(ProviderDataError);
  });

  it('解析 CIK 显式映射、非法映射与缺失 ticker', () => {
    const tickers = [{ cik: '0000320193', ticker: 'AAPL', title: 'Apple Inc.' }];
    const explicit = resolveSecOptions({
      userAgent,
      cikMap: { 'sec:AAPL': '320193' },
    });
    expect(resolveCik(explicit, aapl, tickers)).toBe('0000320193');
    const invalid = resolveSecOptions({ userAgent, cikMap: { 'sec:AAPL': 'abc' } });
    expect(() => resolveCik(invalid, aapl, tickers)).toThrow(ProviderDataError);
    const missing = resolveSecOptions({ userAgent });
    const other = createInstrument({
      assetClass: 'equity',
      venue: 'sec',
      symbol: 'NOPE',
      baseAsset: 'NOPE',
      quoteCurrency: USD,
      tickSize: Decimal.parse('0.01'),
      lotSize: Decimal.ONE,
    }) as Instrument;
    expect(() => resolveCik(missing, other, tickers)).toThrow(ProviderDataError);
  });

  it('拒绝非法选项', () => {
    expect(() => resolveSecOptions({ userAgent: ' ' })).toThrow(ProviderDataError);
    expect(() => resolveSecOptions({ userAgent, id: ' ' })).toThrow(ProviderDataError);
    expect(() => resolveSecOptions({ userAgent, baseUrl: 'not a url' })).toThrow(ProviderDataError);
    expect(() => resolveSecOptions({ userAgent, tickersUrl: 'not a url' })).toThrow(
      ProviderDataError,
    );
  });
});
