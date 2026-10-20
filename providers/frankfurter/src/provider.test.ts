/**
 * @file Frankfurter 提供方测试。
 * @description 用固定 ECB 响应覆盖单条汇率、全部汇率、货币列表与错误。
 * @module frankfurter/provider.test
 */

import { describe, expect, it } from 'vitest';

import {
  BarInterval,
  ProviderDataError,
  ProviderHttpError,
  createCurrency,
  createInstrument,
  type FetchLike,
  type FxInstrument,
} from '@kurumi/finance';
import { Decimal } from '@kurumi/math';

import { createFrankfurterProvider } from './provider.js';

const EUR = createCurrency('EUR', 2);
const USD = createCurrency('USD', 2);
const JPY = createCurrency('JPY', 0);

function fetchFor(routes: Readonly<Record<string, unknown>>): FetchLike {
  return async (input) => {
    const path = new URL(input).pathname;
    const payload = routes[path];
    if (payload === undefined) return new Response('{}', { status: 404 });
    return new Response(JSON.stringify(payload), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    });
  };
}

describe('createFrankfurterProvider', () => {
  it('查询单条汇率', async () => {
    const provider = createFrankfurterProvider({
      fetchImpl: fetchFor({
        '/latest': { amount: 1, base: 'EUR', date: '2026-10-19', rates: { USD: 1.1225 } },
      }),
    });
    const rate = await provider.getRate(EUR, USD);
    expect(rate).toBeDefined();
    expect(rate?.rate.toString()).toBe('1.1225');
    expect(rate?.timestampMs).toBe(Date.UTC(2026, 9, 19));
    expect(rate?.base.code).toBe('EUR');
    expect(rate?.quote.code).toBe('USD');
  });

  it('查询全部汇率与货币列表', async () => {
    const provider = createFrankfurterProvider({
      fetchImpl: fetchFor({
        '/latest': {
          amount: 1,
          base: 'EUR',
          date: '2026-10-19',
          rates: { USD: 1.1225, JPY: 170.1 },
        },
        '/currencies': { EUR: 'Euro', USD: 'US Dollar', JPY: 'Japanese Yen' },
      }),
    });
    const rates = await provider.listRates();
    expect(rates.map((item) => item.quote.code)).toEqual(['JPY', 'USD']);
    expect(rates[0]?.quote.decimals).toBe(0);
    const currencies = await provider.listCurrencies();
    expect(currencies.map((item) => item.code)).toEqual(['EUR', 'JPY', 'USD']);
    expect(currencies.find((item) => item.code === 'JPY')?.decimals).toBe(0);
    expect(JPY.decimals).toBe(0);
  });

  it('获取 FX 日线（参考汇率 OHLC）', async () => {
    const instrument = createInstrument({
      assetClass: 'fx',
      venue: 'fx',
      symbol: 'EUR/USD',
      baseCurrency: EUR,
      quoteCurrency: USD,
      tickSize: Decimal.parse('0.0001'),
      lotSize: Decimal.parse('1000'),
    }) as FxInstrument;
    const provider = createFrankfurterProvider({
      fetchImpl: fetchFor({
        '/2026-10-01..2026-10-02': {
          amount: 1,
          base: 'EUR',
          start_date: '2026-10-01',
          end_date: '2026-10-02',
          rates: {
            '2026-10-01': { USD: 1.1298 },
            '2026-10-02': { USD: 1.1225 },
            '2026-10-03': { USD: 9.9999 },
          },
        },
      }),
    });
    const bars = await provider.getBars(instrument, {
      interval: BarInterval.Day1,
      fromMs: Date.UTC(2026, 9, 1),
      toMs: Date.UTC(2026, 9, 3),
    });
    expect(bars).toHaveLength(2);
    expect(bars[0]?.open.toString()).toBe('1.1298');
    expect(bars[0]?.high.toString()).toBe('1.1298');
    expect(bars[0]?.low.toString()).toBe('1.1298');
    expect(bars[0]?.close.toString()).toBe('1.1298');
    expect(bars[0]?.volume.isZero()).toBe(true);
    await expect(
      provider.getBars(instrument, {
        interval: BarInterval.Hour1,
        fromMs: Date.UTC(2026, 9, 1),
        toMs: Date.UTC(2026, 9, 3),
      }),
    ).rejects.toThrow();
  });

  it('getBars 同时尊重 options.signal 并过滤 toMs 之后的数据', async () => {
    const instrument = createInstrument({
      assetClass: 'fx',
      venue: 'fx',
      symbol: 'EUR/USD',
      baseCurrency: EUR,
      quoteCurrency: USD,
      tickSize: Decimal.parse('0.0001'),
      lotSize: Decimal.parse('1000'),
    }) as FxInstrument;
    const controller = new AbortController();
    controller.abort();
    let wasAborted = false;
    const provider = createFrankfurterProvider({
      fetchImpl: async (_input, init) => {
        wasAborted = init?.signal?.aborted === true;
        throw new Error('aborted');
      },
    });
    await expect(
      provider.getBars(instrument, {
        interval: BarInterval.Day1,
        fromMs: Date.UTC(2026, 9, 1),
        toMs: Date.UTC(2026, 9, 2),
        signal: controller.signal,
      }),
    ).rejects.toBeInstanceOf(ProviderHttpError);
    expect(wasAborted).toBe(true);
  });

  it('处理缺失 base/日期/非法汇率与货币', async () => {
    const missingBase = createFrankfurterProvider({
      fetchImpl: fetchFor({
        '/latest': { date: '2026-10-19', rates: { USD: 1.1 } },
      }),
    });
    const rates = await missingBase.listRates();
    expect(rates[0]?.base.code).toBe('EUR');

    const invalidRate = createFrankfurterProvider({
      fetchImpl: fetchFor({
        '/latest': { base: 'EUR', date: '2026-10-19', rates: { USD: 'abc' } },
      }),
    });
    await expect(invalidRate.listRates()).rejects.toBeInstanceOf(ProviderDataError);

    const invalidDate = createFrankfurterProvider({
      fetchImpl: fetchFor({
        '/latest': { base: 'EUR', date: 'bad-date', rates: { USD: 1.1 } },
      }),
    });
    await expect(invalidDate.listRates()).rejects.toBeInstanceOf(ProviderDataError);

    const missingDate = createFrankfurterProvider({
      fetchImpl: fetchFor({ '/latest': { base: 'EUR', rates: { USD: 1.1 } } }),
    });
    await expect(missingDate.listRates()).rejects.toBeInstanceOf(ProviderDataError);

    const invalidCurrency = createFrankfurterProvider({
      fetchImpl: fetchFor({
        '/latest': { base: 'EUR', date: '2026-10-19', rates: { 'BAD CODE': 1.1 } },
      }),
    });
    await expect(invalidCurrency.listRates()).rejects.toBeInstanceOf(ProviderDataError);

    const invalidCurrencies = createFrankfurterProvider({
      fetchImpl: fetchFor({ '/currencies': { 'BAD CODE': 'Bad' } }),
    });
    await expect(invalidCurrencies.listCurrencies()).rejects.toBeInstanceOf(ProviderDataError);
  });

  it('拒绝非法货币与选项', async () => {
    const provider = createFrankfurterProvider({ fetchImpl: fetchFor({}) });
    await expect(
      provider.getRate({ code: 'USD', decimals: 2 } as never, USD),
    ).rejects.toBeInstanceOf(ProviderDataError);
    expect(() => createFrankfurterProvider({ id: ' ' })).toThrow(ProviderDataError);
    expect(() => createFrankfurterProvider({ baseUrl: 'not a url' })).toThrow(ProviderDataError);
    expect(() =>
      createFrankfurterProvider({ baseCurrency: { code: 'usd', decimals: 2 } as never }),
    ).toThrow(ProviderDataError);
  });

  it('FX K 线拒绝非 FX 标的与倒置区间', async () => {
    const provider = createFrankfurterProvider({ fetchImpl: fetchFor({}) });
    const equity = createInstrument({
      assetClass: 'equity',
      venue: 'nasdaq',
      symbol: 'AAPL',
      baseAsset: 'AAPL',
      quoteCurrency: USD,
      tickSize: Decimal.parse('0.01'),
      lotSize: Decimal.ONE,
    });
    await expect(
      provider.getBars(equity as never, {
        interval: BarInterval.Day1,
        fromMs: Date.UTC(2026, 9, 1),
        toMs: Date.UTC(2026, 9, 10),
      }),
    ).rejects.toBeInstanceOf(ProviderDataError);
    const fx = createInstrument({
      assetClass: 'fx',
      venue: 'fx',
      symbol: 'EUR/USD',
      baseCurrency: EUR,
      quoteCurrency: USD,
      tickSize: Decimal.parse('0.0001'),
      lotSize: Decimal.parse('1000'),
    }) as FxInstrument;
    await expect(
      provider.getBars(fx, {
        interval: BarInterval.Day1,
        fromMs: Date.UTC(2026, 9, 10),
        toMs: Date.UTC(2026, 9, 1),
      }),
    ).rejects.toBeInstanceOf(ProviderDataError);
  });

  it('拒绝同币种与非法响应', async () => {
    const provider = createFrankfurterProvider({
      fetchImpl: fetchFor({
        '/latest': { amount: 1, base: 'EUR', date: '2026-10-19', rates: {} },
        '/2026-10-01..2026-10-02': {
          amount: 1,
          base: 'EUR',
          start_date: '2026-10-01',
          end_date: '2026-10-02',
          rates: {
            '2026-10-01': { USD: 1.1298 },
            '2026-10-02': { USD: 1.1225 },
            '2026-10-03': { USD: 9.9999 },
          },
        },
      }),
    });
    await expect(provider.getRate(EUR, EUR)).rejects.toBeInstanceOf(ProviderDataError);
    await expect(provider.getRate(EUR, USD)).rejects.toBeInstanceOf(ProviderDataError);
  });
});
