/**
 * @file 金融 Agent 的 provider 组合与工具装配。
 * @description 在 example 层按 venue 路由到 Nasdaq/Binance/SEC 等具体 provider；工具包本身不绑定任何 provider。
 * @module example-finance-agent/finance-stack
 */

import {
  AssetClass,
  ProviderDataError,
  createCurrency,
  createInstrument,
  type GetBarsOptions,
  type GetOrderBookOptions,
  type GetTradesOptions,
  type FundamentalDataProvider,
  type Instrument,
  type MarketDataProvider,
  type OptionsDataProvider,
} from '@kurumi/finance';
import { createFinanceTools } from '@kurumi/finance-tools';
import {
  createBinanceFuturesMarketDataProvider,
  createBinanceMarketDataProvider,
} from '@kurumi/binance';
import { createFrankfurterProvider } from '@kurumi/frankfurter';
import { createNasdaqMarketDataProvider, createNasdaqOptionsProvider } from '@kurumi/nasdaq';
import { createSecEdgarFundamentalProvider } from '@kurumi/sec-edgar';
import { createUsTreasuryProvider } from '@kurumi/us-treasury';
import { Decimal } from '@kurumi/math';
import type { ToolDefinition } from '@kurumi/agent';

/**
 * Nasdaq ETF 代码集合。
 *
 * @remarks
 * Nasdaq 公开接口按 `assetclass` 区分股票与 ETF；同一 symbol 用错类别会返回 `data: null`。
 * example 的标的数量有限，因此显式列出；接入更多 ETF 时在此维护。
 */
const NASDAQ_ETF_SYMBOLS: ReadonlySet<string> = new Set(['SPY', 'QQQ', 'DIA', 'IWM']);

/** 组合后的金融工具与标的。 */
export interface FinanceStack {
  /** Agent 工具定义。 */
  readonly tools: readonly ToolDefinition[];
  /** 标的注册表。 */
  readonly instruments: readonly Instrument[];
  /** 工具名称列表，供验证脚本展示。 */
  readonly toolNames: readonly string[];
}

/**
 * 构建真实公开数据源 + 金融工具的 Agent 栈。
 *
 * @param userAgent - SEC 要求的 User-Agent
 * @returns 工具、标的与名称
 */
export function buildFinanceStack(userAgent: string): FinanceStack {
  const usd = createCurrency('USD', 2);
  const usdt = createCurrency('USDT', 8);
  const eur = createCurrency('EUR', 2);

  const btc = createInstrument({
    assetClass: AssetClass.Crypto,
    venue: 'binance',
    symbol: 'BTC/USDT',
    baseAsset: 'BTC',
    quoteCurrency: usdt,
    tickSize: Decimal.parse('0.01'),
    lotSize: Decimal.parse('0.001'),
  });
  const btcPerp = createInstrument({
    assetClass: AssetClass.Perpetual,
    venue: 'binance-futures',
    symbol: 'BTCUSDT',
    quoteCurrency: usdt,
    tickSize: Decimal.parse('0.1'),
    lotSize: Decimal.parse('0.001'),
    fundingIntervalMs: 28_800_000,
  });
  const aapl = equity('nasdaq', 'AAPL', usd);
  const spy = equity('nasdaq', 'SPY', usd);
  const aaplSec = equity('sec', 'AAPL', usd);
  const eurusd = createInstrument({
    assetClass: AssetClass.Fx,
    venue: 'fx',
    symbol: 'EUR/USD',
    baseCurrency: eur,
    quoteCurrency: usd,
    tickSize: Decimal.parse('0.0001'),
    lotSize: Decimal.parse('1000'),
  });
  const instruments = [btc, btcPerp, aapl, spy, aaplSec, eurusd] as const;

  const nasdaqEquity = createNasdaqMarketDataProvider();
  const nasdaqEtf = createNasdaqMarketDataProvider({ assetClass: 'etf' });
  const nasdaqOptions = createNasdaqOptionsProvider({ assetClass: 'etf' });
  const binanceSpot = createBinanceMarketDataProvider();
  const binanceFutures = createBinanceFuturesMarketDataProvider();
  const frankfurter = createFrankfurterProvider();
  const treasury = createUsTreasuryProvider();
  const sec = createSecEdgarFundamentalProvider({ userAgent });

  const marketData = createMarketDataRouter(nasdaqEquity, nasdaqEtf, binanceSpot, binanceFutures);
  const options = createOptionsRouter(nasdaqOptions);
  const fundamentals = createFundamentalRouter(sec);

  const tools = createFinanceTools({
    instruments: [...instruments],
    currencies: { USD: 2, USDT: 8, EUR: 2 },
    marketData,
    options,
    fundamentalData: fundamentals,
    fxRates: frankfurter,
    fxBars: frankfurter,
    yieldCurve: treasury,
  });

  return Object.freeze({
    tools,
    instruments: Object.freeze([...instruments]),
    toolNames: Object.freeze(tools.map((tool) => tool.name)),
  });
}

/** 创建股票标的。 */
function equity(
  venue: string,
  symbol: string,
  currency: ReturnType<typeof createCurrency>,
): Instrument {
  return createInstrument({
    assetClass: AssetClass.Equity,
    venue,
    symbol,
    baseAsset: symbol,
    quoteCurrency: currency,
    tickSize: Decimal.parse('0.01'),
    lotSize: Decimal.ONE,
  });
}

/**
 * 按 venue 路由行情查询。
 *
 * @param nasdaqEquity - Nasdaq 股票行情
 * @param nasdaqEtf - Nasdaq ETF 行情
 * @param binance - Binance 现货
 * @param futures - Binance 期货
 * @returns 组合行情 provider
 */
function createMarketDataRouter(
  nasdaqEquity: MarketDataProvider,
  nasdaqEtf: MarketDataProvider,
  binance: MarketDataProvider,
  futures: MarketDataProvider,
): MarketDataProvider {
  const route = (instrument: Instrument): MarketDataProvider => {
    if (instrument.venue === 'nasdaq') {
      return NASDAQ_ETF_SYMBOLS.has(instrument.symbol) ? nasdaqEtf : nasdaqEquity;
    }
    // SEC 只提供基本面；估值倍数需要同 symbol 的 Nasdaq 行情。
    if (instrument.venue === 'sec') return nasdaqEquity;
    if (instrument.venue === 'binance') return binance;
    if (instrument.venue === 'binance-futures') return futures;
    throw new ProviderDataError('finance-agent-router', `没有行情 provider: ${instrument.venue}`);
  };

  return Object.freeze({
    id: 'finance-agent-market-data',
    getTicker: (instrument: Instrument, signal?: AbortSignal) =>
      requireMethod(route(instrument), 'getTicker')(instrument, signal),
    getQuote: (instrument: Instrument, signal?: AbortSignal) =>
      requireMethod(route(instrument), 'getQuote')(instrument, signal),
    getOrderBook: (instrument: Instrument, options?: GetOrderBookOptions) =>
      requireMethod(route(instrument), 'getOrderBook')(instrument, options),
    getBars: (instrument: Instrument, options: GetBarsOptions) =>
      requireMethod(route(instrument), 'getBars')(instrument, options),
    getTrades: (instrument: Instrument, options?: GetTradesOptions) =>
      requireMethod(route(instrument), 'getTrades')(instrument, options),
  });
}

/** 要求 provider 实现指定方法。 */
function requireMethod<
  TKey extends 'getTicker' | 'getQuote' | 'getOrderBook' | 'getBars' | 'getTrades',
>(provider: MarketDataProvider, key: TKey): NonNullable<MarketDataProvider[TKey]> {
  const method = provider[key];
  if (method === undefined) {
    throw new ProviderDataError(provider.id, `不支持 ${key}`);
  }
  return method.bind(provider) as NonNullable<MarketDataProvider[TKey]>;
}

/** 按 venue 路由期权链。 */
function createOptionsRouter(options: OptionsDataProvider): OptionsDataProvider {
  return Object.freeze({
    id: 'finance-agent-options',
    getExpirations: (instrument: Instrument, signal?: AbortSignal) => {
      if (instrument.venue !== 'nasdaq') {
        throw new ProviderDataError(
          'finance-agent-options',
          `没有期权 provider: ${instrument.venue}`,
        );
      }
      return options.getExpirations(instrument, signal);
    },
    getChain: (instrument: Instrument, expirationMs: number, signal?: AbortSignal) => {
      if (instrument.venue !== 'nasdaq') {
        throw new ProviderDataError(
          'finance-agent-options',
          `没有期权 provider: ${instrument.venue}`,
        );
      }
      return options.getChain(instrument, expirationMs, signal);
    },
  });
}

/** 按 venue 路由基本面。 */
function createFundamentalRouter(provider: FundamentalDataProvider): FundamentalDataProvider {
  return Object.freeze({
    id: 'finance-agent-fundamentals',
    getCompanyProfile: (instrument: Instrument, signal?: AbortSignal) => {
      if (instrument.venue !== 'sec') {
        throw new ProviderDataError(
          'finance-agent-fundamentals',
          `没有基本面 provider: ${instrument.venue}`,
        );
      }
      return provider.getCompanyProfile?.(instrument, signal) ?? Promise.resolve(undefined);
    },
    getFinancialStatements: (
      instrument: Instrument,
      options?: Parameters<FundamentalDataProvider['getFinancialStatements']>[1],
    ) => {
      if (instrument.venue !== 'sec') {
        throw new ProviderDataError(
          'finance-agent-fundamentals',
          `没有基本面 provider: ${instrument.venue}`,
        );
      }
      return provider.getFinancialStatements(instrument, options);
    },
  });
}
