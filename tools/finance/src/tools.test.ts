/**
 * @file 金融 Agent 工具测试。
 * @description 覆盖 provider 注入工具、分析工具、注册表、错误与 K 线指标。
 * @module finance-tools/tools.test
 */

import { describe, expect, it } from 'vitest';

import { createToolRegistry, type ToolRegistry } from '@kurumi/agent';
import {
  BarInterval,
  OptionType,
  createBar,
  StatementPeriodType,
  createCompanyProfile,
  createCurrency,
  createFinancialStatements,
  createFxRate,
  createInstrument,
  createOrderBook,
  createQuote,
  createTicker,
  createTrade,
  type FxInstrument,
  type Instrument,
  type OptionContractQuote,
} from '@kurumi/finance';
import { Decimal } from '@kurumi/math';

import { FinanceToolError } from './errors.js';
import { createFinanceProviderTools, createFinanceToolRegistry, parseDateUtc } from './index.js';

const T0 = Date.UTC(2026, 9, 19);
const DAY_MS = 86_400_000;
const USD = createCurrency('USD', 2);
const USDT = createCurrency('USDT', 8);
const EUR = createCurrency('EUR', 2);

const btc = createInstrument({
  assetClass: 'crypto',
  venue: 'binance',
  symbol: 'BTC/USDT',
  baseAsset: 'BTC',
  quoteCurrency: USDT,
  tickSize: Decimal.parse('0.01'),
  lotSize: Decimal.parse('0.001'),
}) as Instrument;
const eurusd = createInstrument({
  assetClass: 'fx',
  venue: 'fx',
  symbol: 'EUR/USD',
  baseCurrency: EUR,
  quoteCurrency: USD,
  tickSize: Decimal.parse('0.0001'),
  lotSize: Decimal.parse('1000'),
}) as FxInstrument;
const aapl = createInstrument({
  assetClass: 'equity',
  venue: 'sec',
  symbol: 'AAPL',
  baseAsset: 'AAPL',
  quoteCurrency: USD,
  tickSize: Decimal.parse('0.01'),
  lotSize: Decimal.ONE,
}) as Instrument;

function bars(instrument: Instrument, count = 30) {
  return Array.from({ length: count }, (_, index) => {
    const close = 100 + index * 0.5;
    const openTimeMs = T0 + index * DAY_MS;
    return createBar({
      instrument,
      interval: BarInterval.Day1,
      openTimeMs,
      closeTimeMs: openTimeMs + DAY_MS,
      open: Decimal.parse(String(close - 0.25)),
      high: Decimal.parse(String(close + 0.5)),
      low: Decimal.parse(String(close - 0.5)),
      close: Decimal.parse(String(close)),
      volume: Decimal.parse('10'),
    });
  });
}

const btcBars = bars(btc);
const fxBars = bars(eurusd);

const providers = {
  marketData: {
    id: 'fake',
    getTicker: async (instrument: Instrument) =>
      createTicker({
        instrument,
        lastPrice: Decimal.parse('100'),
        bid: Decimal.parse('99.99'),
        ask: Decimal.parse('100.01'),
        volume24h: Decimal.parse('1000'),
        timestampMs: T0,
      }),
    getQuote: async (instrument: Instrument) =>
      createQuote({
        instrument,
        bid: Decimal.parse('99.99'),
        ask: Decimal.parse('100.01'),
        bidSize: Decimal.parse('1'),
        askSize: Decimal.parse('2'),
        timestampMs: T0,
      }),
    getOrderBook: async (instrument: Instrument) =>
      createOrderBook({
        instrument,
        bids: [{ price: Decimal.parse('99.99'), quantity: Decimal.parse('1') }],
        asks: [{ price: Decimal.parse('100.01'), quantity: Decimal.parse('2') }],
        timestampMs: T0,
        sequence: 1,
      }),
    getBars: async (instrument: Instrument) => (instrument.assetClass === 'fx' ? fxBars : btcBars),
    getTrades: async (instrument: Instrument) => [
      createTrade({
        instrument,
        price: Decimal.parse('100'),
        quantity: Decimal.parse('1'),
        side: 'buy',
        timestampMs: T0,
      }),
    ],
  },
  fxRates: {
    id: 'fake-fx',
    getRate: async () =>
      createFxRate({ base: EUR, quote: USD, rate: Decimal.parse('1.1'), timestampMs: T0 }),
    listRates: async () => [
      createFxRate({ base: EUR, quote: USD, rate: Decimal.parse('1.1'), timestampMs: T0 }),
    ],
  },
  fxBars: {
    id: 'fake-fx-bars',
    getBars: async () => fxBars,
  },
  options: {
    id: 'fake-options',
    getExpirations: async () => [T0 + 30 * DAY_MS],
    getChain: async () => {
      const contract: OptionContractQuote = Object.freeze({
        contractId: 'fake-call',
        optionType: OptionType.Call,
        strike: Decimal.parse('100'),
        expirationMs: T0 + 30 * DAY_MS,
        bid: Decimal.parse('5'),
        ask: Decimal.parse('5.2'),
        timestampMs: T0,
      });
      return Object.freeze({
        underlying: btc,
        expirationMs: T0 + 30 * DAY_MS,
        contracts: Object.freeze([contract]),
        timestampMs: T0,
      });
    },
  },
  yieldCurve: {
    id: 'fake-curve',
    getLatest: async () => ({
      date: '2026-10-19',
      currency: USD,
      points: [{ tenorYears: 10, yieldRate: Decimal.parse('0.05') }],
    }),
    getCurve: async () => ({
      date: '2026-10-18',
      currency: USD,
      points: [{ tenorYears: 10, yieldRate: Decimal.parse('0.051') }],
    }),
  },
  fundamentalData: {
    id: 'fake-fundamentals',
    getCompanyProfile: async () =>
      createCompanyProfile({
        instrument: aapl,
        name: 'Apple Inc.',
        currency: USD,
        country: 'US',
      }),
    getFinancialStatements: async (_instrument: Instrument, options?: { periods?: number }) =>
      [sampleStatements(2025), sampleStatements(2024)].slice(0, options?.periods ?? 5),
  },
  referenceData: {
    id: 'fake-reference',
    listInstruments: async (options?: { venue?: string; assetClass?: string }) =>
      [btc, aapl].filter(
        (instrument) =>
          (options?.venue === undefined || instrument.venue === options.venue) &&
          (options?.assetClass === undefined || instrument.assetClass === options.assetClass),
      ),
    getInstrument: async (key: string) =>
      [btc, aapl].find((instrument) => instrument.venue + ':' + instrument.symbol === key),
    listCurrencies: async () => [USDT, USD],
  },
};

function sampleStatements(fiscalYear = 2025) {
  return createFinancialStatements({
    period: {
      fiscalYear,
      periodType: StatementPeriodType.Annual,
      startMs: Date.UTC(fiscalYear, 0, 1),
      endMs: Date.UTC(fiscalYear, 11, 31),
      currency: USD,
    },
    income: {
      revenue: Decimal.parse('1000'),
      costOfRevenue: Decimal.parse('600'),
      operatingExpenses: Decimal.parse('200'),
      interestExpense: Decimal.parse('20'),
      taxExpense: Decimal.parse('36'),
      sharesOutstanding: Decimal.parse('100'),
    },
    balance: {
      cashAndEquivalents: Decimal.parse('100'),
      shortTermInvestments: Decimal.parse('50'),
      accountsReceivable: Decimal.parse('150'),
      inventory: Decimal.parse('100'),
      otherCurrentAssets: Decimal.parse('50'),
      propertyPlantEquipment: Decimal.parse('500'),
      goodwill: Decimal.parse('100'),
      intangibleAssets: Decimal.parse('50'),
      otherNonCurrentAssets: Decimal.parse('50'),
      accountsPayable: Decimal.parse('100'),
      shortTermDebt: Decimal.parse('50'),
      otherCurrentLiabilities: Decimal.parse('50'),
      longTermDebt: Decimal.parse('300'),
      otherNonCurrentLiabilities: Decimal.parse('50'),
      shareholdersEquity: Decimal.parse('600'),
    },
    cashFlow: {
      operatingCashFlow: Decimal.parse('180'),
      capitalExpenditure: Decimal.parse('50'),
      investingCashFlow: Decimal.parse('-100'),
      financingCashFlow: Decimal.parse('-20'),
      depreciationAmortization: Decimal.parse('40'),
      dividendsPaid: Decimal.parse('30'),
    },
  });
}

function registry(extra: Record<string, unknown> = {}): ToolRegistry {
  return createFinanceToolRegistry({
    instruments: [btc, eurusd, aapl],
    currencies: { USDT: 8 },
    ...providers,
    ...extra,
  });
}

function rawRegistry(extra: Record<string, unknown> = {}): ToolRegistry {
  return createToolRegistry([
    ...createFinanceProviderTools({
      instruments: [btc, eurusd, aapl],
      currencies: { USDT: 8 },
      ...providers,
      ...extra,
    }),
  ]);
}

async function execute(toolRegistry: ToolRegistry, name: string, input: unknown): Promise<unknown> {
  const tool = toolRegistry.get(name);
  if (tool === undefined) throw new Error(`missing tool ${name}`);
  const parsed = tool.parseInput === undefined ? input : tool.parseInput(input as never);
  const result = await tool.execute(parsed as never, {
    callId: 'call-1',
    name,
    signal: new AbortController().signal,
    messages: [],
  });
  if (typeof result === 'string') return JSON.parse(result);
  return result.metadata;
}

describe('provider tools', () => {
  it('注册行情/参考/基本面/FX/期权/收益率曲线工具', () => {
    const tools = rawRegistry();
    const names = tools.descriptors.map((descriptor) => descriptor.name);
    for (const name of [
      'finance_get_ticker',
      'finance_get_quote',
      'finance_get_order_book',
      'finance_get_bars',
      'finance_get_trades',
      'finance_list_instruments',
      'finance_get_instrument',
      'finance_get_company_profile',
      'finance_get_financial_statements',
      'finance_get_fx_rate',
      'finance_list_fx_rates',
      'finance_get_fx_bars',
      'finance_get_option_expirations',
      'finance_get_options_chain',
      'finance_get_yield_curve',
    ]) {
      expect(names).toContain(name);
    }
  });

  it('执行行情与 K 线指标工具', async () => {
    const tools = rawRegistry();
    const ticker = (await execute(tools, 'finance_get_ticker', {
      instrument: 'binance:BTC/USDT',
    })) as { lastPrice: string };
    expect(ticker.lastPrice).toBe('100');

    const bars = (await execute(tools, 'finance_get_bars', {
      instrument: 'binance:BTC/USDT',
      interval: '1d',
      fromMs: T0,
      toMs: T0 + 30 * DAY_MS,
    })) as unknown[];
    expect(bars).toHaveLength(30);
  });

  it('执行 quote/order book/trades/参考数据工具', async () => {
    const tools = rawRegistry();
    const quote = (await execute(tools, 'finance_get_quote', {
      instrument: 'binance:BTC/USDT',
    })) as { bid: string; ask: string };
    expect(quote.bid).toBe('99.99');
    expect(quote.ask).toBe('100.01');

    const book = (await execute(tools, 'finance_get_order_book', {
      instrument: 'binance:BTC/USDT',
      depth: 5,
    })) as { bids: unknown[]; asks: unknown[] };
    expect(book.bids).toHaveLength(1);
    expect(book.asks).toHaveLength(1);

    const trades = (await execute(tools, 'finance_get_trades', {
      instrument: 'binance:BTC/USDT',
      limit: 1,
    })) as unknown[];
    expect(trades).toHaveLength(1);

    const listed = (await execute(tools, 'finance_list_instruments', {
      venue: 'binance',
      assetClass: 'crypto',
    })) as unknown[];
    expect(listed).toHaveLength(1);
    const instrument = (await execute(tools, 'finance_get_instrument', {
      key: 'binance:BTC/USDT',
    })) as { symbol: string };
    expect(instrument.symbol).toBe('BTC/USDT');
    const rates = (await execute(tools, 'finance_list_fx_rates', {})) as unknown[];
    expect(rates).toHaveLength(1);
    const datedCurve = (await execute(tools, 'finance_get_yield_curve', {
      currency: 'USD',
      date: '2026-10-18',
    })) as { date: string };
    expect(datedCurve.date).toBe('2026-10-18');
  });

  it('执行 FX/期权/收益率曲线/基本面工具', async () => {
    const tools = rawRegistry();
    const fx = (await execute(tools, 'finance_get_fx_rate', {
      base: 'EUR',
      quote: 'USD',
    })) as { rate: string };
    expect(fx.rate).toBe('1.1');

    const fxBarsResult = (await execute(tools, 'finance_get_fx_bars', {
      instrument: 'fx:EUR/USD',
      interval: '1d',
      fromMs: T0,
      toMs: T0 + 30 * DAY_MS,
    })) as unknown[];
    expect(fxBarsResult).toHaveLength(30);

    const expirations = (await execute(tools, 'finance_get_option_expirations', {
      instrument: 'binance:BTC/USDT',
    })) as Array<{ expirationMs: number }>;
    expect(expirations[0]?.expirationMs).toBe(T0 + 30 * DAY_MS);

    const chain = (await execute(tools, 'finance_get_options_chain', {
      instrument: 'binance:BTC/USDT',
      expiration: '2026-11-18',
    })) as { contracts: unknown[] };
    expect(chain.contracts).toHaveLength(1);

    const curve = (await execute(tools, 'finance_get_yield_curve', {
      currency: 'USD',
    })) as { date: string };
    expect(curve.date).toBe('2026-10-19');

    const profile = (await execute(tools, 'finance_get_company_profile', {
      instrument: 'sec:AAPL',
    })) as { name: string };
    expect(profile.name).toBe('Apple Inc.');

    const statements = (await execute(tools, 'finance_get_financial_statements', {
      instrument: 'sec:AAPL',
      periods: 1,
    })) as Array<{ income: { revenue: string } }>;
    expect(statements).toHaveLength(1);
    expect(statements[0]?.income.revenue).toBe('1000');
  });

  it('未提供 provider 时只注册分析工具', () => {
    const tools = createFinanceToolRegistry({ instruments: [btc] });
    expect(tools.descriptors.some((item) => item.name.startsWith('finance_get_'))).toBe(false);
    expect(tools.has('finance_option_price')).toBe(true);
  });

  it('默认预设同时包含复合工具与低层 provider 查询工具', () => {
    const tools = createFinanceToolRegistry({
      instruments: [btc],
      marketData: providers.marketData,
    });
    expect(tools.has('finance_market_analysis')).toBe(true);
    expect(tools.has('finance_get_ticker')).toBe(true);
  });

  it('可按分组关闭 provider 查询工具，控制模型可选工具数量', () => {
    const tools = createFinanceToolRegistry({
      instruments: [btc],
      marketData: providers.marketData,
      shouldIncludeProviderTools: false,
    });
    expect(tools.has('finance_market_analysis')).toBe(true);
    expect(tools.has('finance_get_ticker')).toBe(false);
  });

  it('未知标的/货币与缺失汇率失败', async () => {
    const tools = rawRegistry();
    await expect(
      execute(tools, 'finance_get_ticker', { instrument: 'nope' }),
    ).rejects.toBeInstanceOf(FinanceToolError);
    await expect(
      execute(tools, 'finance_get_fx_rate', { base: 'BAD CODE', quote: 'USD' }),
    ).rejects.toThrow();
    await expect(
      execute(tools, 'finance_get_fx_bars', {
        instrument: 'binance:BTC/USDT',
        interval: '1d',
        fromMs: T0,
        toMs: T0 + DAY_MS,
      }),
    ).rejects.toBeInstanceOf(FinanceToolError);
    const missingRate = rawRegistry({
      instruments: [eurusd],
      fxRates: {
        ...providers.fxRates,
        getRate: async () => undefined,
      },
    });
    await expect(
      execute(missingRate, 'finance_get_fx_rate', { base: 'EUR', quote: 'USD' }),
    ).rejects.toBeInstanceOf(FinanceToolError);
  });
});

describe('composite tools', () => {
  it('一次完成行情查询、指标计算与盘口摘要', async () => {
    const tools = registry();
    expect(tools.descriptors.map((item) => item.name)).toEqual(
      expect.arrayContaining([
        'finance_market_analysis',
        'finance_fundamental_analysis',
        'finance_fx_analysis',
        'finance_option_analysis',
        'finance_yield_curve',
      ]),
    );
    const market = (await execute(tools, 'finance_market_analysis', {
      instrument: 'binance:BTC/USDT',
      interval: '1d',
      lookbackBars: 30,
      includeOrderBook: true,
    })) as {
      ticker: { lastPrice: string };
      lastBars: unknown[];
      indicators: { rsi: number | null; macd: { histogram: number | null } };
      orderBook: { bids: unknown[] };
    };
    expect(market.ticker.lastPrice).toBe('100');
    expect(market.lastBars).toHaveLength(5);
    expect(market.indicators.rsi).not.toBeNull();
    expect(market.orderBook.bids).toHaveLength(1);

    const fundamentals = (await execute(tools, 'finance_fundamental_analysis', {
      instrument: 'sec:AAPL',
      periods: 2,
    })) as {
      ratios: { netMargin: string };
      period: { fiscalYear: number };
      prior: { period: { fiscalYear: number }; ratios: { netMargin: string } } | null;
      growth: { revenueGrowth: string } | null;
      piotroski: { score: number } | null;
      valuation: { marketCap: { amount: string } } | null;
    };
    expect(fundamentals.period.fiscalYear).toBe(2025);
    expect(fundamentals.ratios.netMargin).toBe('0.144');
    expect(fundamentals.prior?.period.fiscalYear).toBe(2024);
    expect(fundamentals.prior?.ratios.netMargin).toBe('0.144');
    expect(fundamentals.growth?.revenueGrowth).toBe('0');
    expect(fundamentals.piotroski?.score).toBeGreaterThanOrEqual(0);
    expect(fundamentals.valuation?.marketCap.amount).toBe('10000');

    const fx = (await execute(tools, 'finance_fx_analysis', {
      instrument: 'fx:EUR/USD',
      lookbackDays: 30,
    })) as { rate: { rate: string }; indicators: { rsi: number | null } };
    expect(fx.rate.rate).toBe('1.1');
    expect(fx.indicators.rsi).not.toBeNull();

    const options = (await execute(tools, 'finance_option_analysis', {
      instrument: 'binance:BTC/USDT',
      maxContracts: 4,
    })) as { totalContracts: number; contracts: unknown[]; expirationDate: string };
    expect(options.totalContracts).toBe(1);
    expect(options.contracts).toHaveLength(1);
    expect(options.expirationDate).toBe('2026-11-18');

    const curve = (await execute(tools, 'finance_yield_curve', {
      currency: 'USD',
    })) as { date: string; tenYear: { yieldRate: string } };
    expect(curve.date).toBe('2026-10-19');
    expect(curve.tenYear.yieldRate).toBe('0.05');

    const datedCurve = (await execute(tools, 'finance_yield_curve', {
      currency: 'USD',
      date: '2026-10-18',
    })) as { date: string };
    expect(datedCurve.date).toBe('2026-10-18');

    const withBars = (await execute(tools, 'finance_market_analysis', {
      instrument: 'binance:BTC/USDT',
      interval: '1d',
      lookbackBars: 30,
      includeBars: true,
    })) as { bars: unknown[] };
    expect(withBars.bars).toHaveLength(30);
  });

  it('区间内没有 K 线时返回空摘要而不是失败', async () => {
    const tools = createFinanceToolRegistry({
      instruments: [btc],
      marketData: {
        id: 'empty-bars',
        getBars: async () => [],
      },
    });
    const result = (await execute(tools, 'finance_market_analysis', {
      instrument: 'binance:BTC/USDT',
      interval: '1d',
      lookbackBars: 30,
    })) as { lastBars: unknown[]; indicators: unknown };
    expect(result.lastBars).toEqual([]);
    expect(result.indicators).toBeNull();
  });

  it('FX/期权复合工具在缺少部分能力时仍可用', async () => {
    const ratesOnly = createFinanceToolRegistry({
      instruments: [eurusd],
      fxRates: providers.fxRates,
    });
    const fxRateOnly = (await execute(ratesOnly, 'finance_fx_analysis', {
      instrument: 'fx:EUR/USD',
    })) as { rate: { rate: string }; lastBars: unknown; indicators: unknown };
    expect(fxRateOnly.rate.rate).toBe('1.1');
    expect(fxRateOnly.lastBars).toBeNull();
    expect(fxRateOnly.indicators).toBeNull();

    const barsOnly = createFinanceToolRegistry({
      instruments: [eurusd],
      fxBars: providers.fxBars,
    });
    const fxBarsOnly = (await execute(barsOnly, 'finance_fx_analysis', {
      instrument: 'fx:EUR/USD',
    })) as { rate: unknown; indicators: { rsi: number | null } };
    expect(fxBarsOnly.rate).toBeNull();
    expect(fxBarsOnly.indicators.rsi).not.toBeNull();

    const optionsOnly = createFinanceToolRegistry({
      instruments: [btc],
      options: providers.options,
    });
    const optionAnalysis = (await execute(optionsOnly, 'finance_option_analysis', {
      instrument: 'binance:BTC/USDT',
      expiration: '2026-11-18',
      maxContracts: 2,
    })) as { underlyingPrice: unknown; contracts: unknown[] };
    expect(optionAnalysis.underlyingPrice).toBeNull();
    expect(optionAnalysis.contracts).toHaveLength(1);

    const tickerOnly = createFinanceToolRegistry({
      instruments: [btc],
      marketData: {
        id: 'ticker-only',
        getTicker: providers.marketData.getTicker,
      },
    });
    const minimal = (await execute(tickerOnly, 'finance_market_analysis', {
      instrument: 'binance:BTC/USDT',
    })) as { lastBars: unknown; indicators: unknown; ticker: { lastPrice: string } };
    expect(minimal.ticker.lastPrice).toBe('100');
    expect(minimal.lastBars).toBeNull();
    expect(minimal.indicators).toBeNull();
  });
});

describe('analysis tools', () => {
  it('严格校验 UTC 日期，不接受自动归一化', () => {
    expect(parseDateUtc('2026-10-19')).toBe(Date.UTC(2026, 9, 19));
    expect(() => parseDateUtc('2026-13-01')).toThrow(FinanceToolError);
    expect(() => parseDateUtc('2026-02-31')).toThrow(FinanceToolError);
  });

  it('schema 与实际 Decimal 解析器保持一致，拒绝 "1."', () => {
    const tool = registry().get('finance_money_calculate');
    expect(tool).toBeDefined();
    expect(() =>
      tool?.parseInput?.({
        operation: 'format',
        left: '1.',
        currency: 'USD',
      } as never),
    ).toThrow();
  });

  it('金额计算、格式化与除零错误', async () => {
    const tools = registry();
    const sum = (await execute(tools, 'finance_money_calculate', {
      operation: 'add',
      left: '10.50',
      right: '2.25',
      currency: 'USD',
    })) as { result: string };
    expect(sum.result).toBe('12.75');
    const difference = (await execute(tools, 'finance_money_calculate', {
      operation: 'subtract',
      left: '10.50',
      right: '2.25',
      currency: 'USD',
    })) as { result: string };
    expect(difference.result).toBe('8.25');
    const product = (await execute(tools, 'finance_money_calculate', {
      operation: 'multiply',
      left: '10',
      right: '2.5',
      currency: 'USD',
    })) as { result: string };
    expect(product.result).toBe('25');
    const quotient = (await execute(tools, 'finance_money_calculate', {
      operation: 'divide',
      left: '10',
      right: '4',
      currency: 'USD',
    })) as { result: string };
    expect(quotient.result).toBe('2.5');
    const comparison = (await execute(tools, 'finance_money_calculate', {
      operation: 'compare',
      left: '10',
      right: '9',
      currency: 'USD',
    })) as { comparison: number };
    expect(comparison.comparison).toBe(1);
    const formatted = (await execute(tools, 'finance_money_calculate', {
      operation: 'format',
      left: '10',
      currency: 'USD',
    })) as { formatted: string };
    expect(formatted.formatted).toBe('10.00 USD');
    await expect(
      execute(tools, 'finance_money_calculate', {
        operation: 'divide',
        left: '1',
        right: '0',
        currency: 'USD',
      }),
    ).rejects.toThrow();
    await expect(
      execute(tools, 'finance_money_calculate', {
        operation: 'add',
        left: '1',
        currency: 'USD',
      }),
    ).rejects.toBeInstanceOf(FinanceToolError);
  });

  it('期权定价与隐含波动率', async () => {
    const tools = registry();
    const price = (await execute(tools, 'finance_option_price', {
      spot: 100,
      strike: 100,
      timeToExpiryYears: 1,
      volatility: 0.2,
      riskFreeRate: 0.05,
      dividendYield: 0.01,
      optionType: 'call',
    })) as { price: number };
    expect(price.price).toBeGreaterThan(0);
    const iv = (await execute(tools, 'finance_option_implied_volatility', {
      spot: 100,
      strike: 100,
      timeToExpiryYears: 1,
      riskFreeRate: 0.05,
      dividendYield: 0.01,
      marketPrice: price.price,
      optionType: 'call',
    })) as { volatility: number };
    expect(iv.volatility).toBeCloseTo(0.2, 5);
  });

  it('债券分析、绩效指标、技术指标', async () => {
    const tools = registry();
    const bond = (await execute(tools, 'finance_bond_analyze', {
      faceValue: '1000',
      couponRate: '0.05',
      couponFrequency: 2,
      issueDate: '2020-01-01',
      maturityDate: '2030-01-01',
      dayCount: 'actual/actual',
      currency: 'USD',
      yieldRate: '0.06',
    })) as {
      couponPerPeriod: { amount: string };
      yieldRate: string;
      dv01: { amount: string };
      cleanPrice: { amount: string };
    };
    expect(bond.couponPerPeriod.amount).toBe('25');
    expect(bond.yieldRate).toBe('0.06');
    expect(Number(bond.dv01.amount)).toBeGreaterThan(0);

    const bondFromPrice = (await execute(tools, 'finance_bond_analyze', {
      faceValue: '1000',
      couponRate: '0.05',
      couponFrequency: 2,
      issueDate: '2020-01-01',
      maturityDate: '2030-01-01',
      dayCount: 'actual/actual',
      currency: 'USD',
      cleanPrice: bond.cleanPrice.amount,
    })) as { yieldRate: string };
    expect(Number(bondFromPrice.yieldRate)).toBeGreaterThan(0);
    await expect(
      execute(tools, 'finance_bond_analyze', {
        faceValue: '1000',
        couponRate: '0.05',
        couponFrequency: 2,
        issueDate: '2020-01-01',
        maturityDate: '2030-01-01',
        dayCount: 'actual/actual',
        currency: 'USD',
      }),
    ).rejects.toBeInstanceOf(FinanceToolError);

    const metrics = (await execute(tools, 'finance_performance_metrics', {
      prices: [100, 110, 99, 120],
      periodsPerYear: 12,
      riskFreeRate: 0.02,
    })) as { annualizedReturn: number; maxDrawdown: { maxDrawdown: number } };
    expect(metrics.annualizedReturn).toBeCloseTo(1.0736, 6);
    expect(metrics.maxDrawdown.maxDrawdown).toBeCloseTo(-0.1, 8);
    const flatMetrics = (await execute(tools, 'finance_performance_metrics', {
      prices: [100, 100, 100],
      periodsPerYear: 12,
    })) as { sharpeRatio: number | null };
    expect(flatMetrics.sharpeRatio).toBeNull();
  });

  it('公司行为、持仓 PnL 与风控', async () => {
    const tools = registry();
    const adjusted = (await execute(tools, 'finance_corporate_action_adjust', {
      prices: [
        { timestampMs: T0, price: '100' },
        { timestampMs: T0 + 2 * DAY_MS, price: '50' },
      ],
      actions: [{ type: 'split', exDateMs: T0 + DAY_MS, ratio: '2' }],
    })) as Array<{ price: string }>;
    expect(adjusted[0]?.price).toBe('50');
    expect(adjusted[1]?.price).toBe('50');

    const stockDividend = (await execute(tools, 'finance_corporate_action_adjust', {
      prices: [{ timestampMs: T0, price: '100' }],
      actions: [{ type: 'stock_dividend', exDateMs: T0 + DAY_MS, ratio: '0.1' }],
    })) as Array<{ price: string }>;
    expect(stockDividend[0]?.price).toBe('90.909090909090909091');
    const cashDividend = (await execute(tools, 'finance_corporate_action_adjust', {
      prices: [{ timestampMs: T0, price: '100' }],
      actions: [
        {
          type: 'cash_dividend',
          exDateMs: T0 + DAY_MS,
          amountPerShare: '1',
          currency: 'USD',
        },
      ],
    })) as Array<{ price: string }>;
    expect(cashDividend[0]?.price).toBe('100');
    await expect(
      execute(tools, 'finance_corporate_action_adjust', {
        prices: [{ timestampMs: T0, price: '100' }],
        actions: [{ type: 'split', exDateMs: T0 + DAY_MS }],
      }),
    ).rejects.toBeInstanceOf(FinanceToolError);

    const pnl = (await execute(tools, 'finance_position_pnl', {
      instrument: 'binance:BTC/USDT',
      fills: [
        { side: 'buy', price: '100', quantity: '1', timestampMs: T0 + 1 },
        { side: 'sell', price: '110', quantity: '1', timestampMs: T0 + 2 },
      ],
      markPrice: '110',
    })) as {
      position: { netQuantity: string; grossRealizedPnl: { amount: string } };
    };
    expect(pnl.position.netQuantity).toBe('0');
    expect(pnl.position.grossRealizedPnl.amount).toBe('10');

    const openPnl = (await execute(tools, 'finance_position_pnl', {
      instrument: 'binance:BTC/USDT',
      fills: [{ side: 'buy', price: '100', quantity: '1', timestampMs: T0 + 1 }],
    })) as { position: { netQuantity: string } };
    expect(openPnl.position.netQuantity).toBe('1');

    const risk = (await execute(tools, 'finance_risk_check', {
      instrument: 'binance:BTC/USDT',
      side: 'buy',
      quantity: '1',
      price: '100',
      limits: { maxOrderNotional: { amount: '50', currency: 'USDT' } },
    })) as Array<{ code: string }>;
    expect(risk[0]?.code).toBe('order_notional_exceeded');

    const positionRisk = (await execute(tools, 'finance_risk_check', {
      instrument: 'binance:BTC/USDT',
      side: 'buy',
      quantity: '1',
      price: '100',
      limits: {
        maxPositionQuantity: '0.5',
        maxPositionNotional: { amount: '50', currency: 'USDT' },
        allowedInstrumentKeys: ['other'],
      },
    })) as Array<{ code: string }>;
    expect(positionRisk.map((item) => item.code)).toContain('instrument_not_allowed');
    expect(positionRisk.map((item) => item.code)).toContain('position_quantity_exceeded');
  });
});

describe('tool schema descriptions', () => {
  it('每个工具输入字段都有非空 describe', () => {
    const raw = rawRegistry();
    const composite = registry();
    const tools = [...raw.descriptors, ...composite.descriptors];
    for (const tool of tools) {
      assertDescribed(tool.inputSchema, `${tool.name}.inputSchema`);
    }
  });
});

/** 递归校验 JSON Schema 的每个属性都有 description。 */
function assertDescribed(schema: unknown, path: string): void {
  if (schema === null || typeof schema !== 'object') return;
  const record = schema as Record<string, unknown>;
  const properties = record['properties'];
  if (properties !== null && typeof properties === 'object' && !Array.isArray(properties)) {
    for (const [key, child] of Object.entries(properties)) {
      if (child === null || typeof child !== 'object') {
        throw new Error(`${path}.${key} 不是对象 schema`);
      }
      const description = (child as Record<string, unknown>)['description'];
      if (typeof description !== 'string' || description.trim() === '') {
        throw new Error(`${path}.${key} 缺少 description`);
      }
      assertDescribed(child, `${path}.${key}`);
    }
  }
  const items = record['items'];
  if (items !== undefined) assertDescribed(items, `${path}[]`);
  for (const key of ['anyOf', 'oneOf', 'allOf'] as const) {
    const branches = record[key];
    if (Array.isArray(branches)) {
      branches.forEach((branch, index) => assertDescribed(branch, `${path}.${key}[${index}]`));
    }
  }
}
