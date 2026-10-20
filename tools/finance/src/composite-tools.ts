/**
 * @file 复合金融工具：一次调用完成“查询 + 计算 + 摘要”。
 * @description 减少 Agent 多轮 tool_use 的参数搬运；provider 仍由调用方注入，本文件不绑定具体数据源。
 * @module finance-tools/composite-tools
 */

import { createTool, type ToolDefinition } from '@kurumi/agent';
import {
  BarInterval,
  barIntervalMs,
  computeBarIndicators,
  computeDuPont,
  computeFundamentalRatios,
  computeGrowthMetrics,
  computePiotroskiFScore,
  computeValuationMultiples,
  type BarIndicatorSeries,
  type FxInstrument,
} from '@kurumi/finance';
import { Decimal } from '@kurumi/math';
import { boolean, number, object } from '@kurumi/schema';

import { FinanceToolError } from './errors.js';
import { toolResult } from './json.js';
import { createToolContext, type FinanceProviderToolOptions } from './options.js';
import { parseDateUtc } from './provider-tools.js';
import {
  CURRENCY_CODE_SCHEMA,
  DATE_SCHEMA,
  INSTRUMENT_KEY_SCHEMA,
  INTERVAL_SCHEMA,
  PERIOD_TYPE_SCHEMA,
  POSITIVE_INTEGER_SCHEMA,
} from './schemas.js';

/** 单根 K 线最多返回条数的默认值。 */
const DEFAULT_LOOKBACK_BARS = 120;

/** FX 日线默认回看天数。 */
const DEFAULT_FX_LOOKBACK_DAYS = 60;

/** 期权分析默认最多合约数。 */
const DEFAULT_MAX_OPTION_CONTRACTS = 20;

/**
 * 创建复合金融工具。
 *
 * @param options - provider 注入、标的/货币与时间配置
 * @returns 只读工具定义数组；未提供的 provider 不会生成对应工具
 */
export function createFinanceCompositeTools(
  options: FinanceProviderToolOptions,
): readonly ToolDefinition[] {
  const context = createToolContext(options);
  const nowMs = options.nowMs ?? (() => Date.now());
  const tools: ToolDefinition[] = [];

  const marketData = options.marketData;
  if (marketData !== undefined) {
    tools.push(
      createTool({
        name: 'finance_market_analysis',
        description:
          '一次完成行情查询与技术指标计算：Ticker/报价/盘口 + K 线 + SMA/EMA/RSI/MACD/布林带/ATR/随机/VWAP 摘要。',
        inputSchema: object({
          instrument: INSTRUMENT_KEY_SCHEMA.describe('标的 key，例如 binance:BTC/USDT'),
          interval: INTERVAL_SCHEMA.describe('K 线周期，缺省 1d').default('1d'),
          lookbackBars: number({ isInteger: true, min: 5, max: 1_000 })
            .describe('回看 K 线根数，缺省 120')
            .default(DEFAULT_LOOKBACK_BARS),
          includeBars: boolean()
            .describe('是否附带原始 K 线数组；缺省 false，仅返回指标摘要')
            .default(false),
          includeOrderBook: boolean().describe('是否附带盘口前 5 档；缺省 false').default(false),
        }),
        execute: async (
          { instrument, interval, lookbackBars, includeBars, includeOrderBook },
          { signal },
        ) => {
          const resolved = context.resolveInstrument(instrument);
          const intervalValue = interval as BarInterval;
          const toMs = nowMs();
          const fromMs = toMs - barIntervalMs(intervalValue) * lookbackBars;
          const [ticker, quote, bars, orderBook] = await Promise.all([
            marketData.getTicker === undefined
              ? Promise.resolve(undefined)
              : marketData.getTicker(resolved, signal),
            marketData.getQuote === undefined
              ? Promise.resolve(undefined)
              : marketData.getQuote(resolved, signal),
            marketData.getBars === undefined
              ? Promise.resolve(undefined)
              : marketData.getBars(resolved, {
                  interval: intervalValue,
                  fromMs,
                  toMs,
                  signal,
                }),
            includeOrderBook && marketData.getOrderBook !== undefined
              ? marketData.getOrderBook(resolved, { depth: 5, signal })
              : Promise.resolve(undefined),
          ]);
          const indicators =
            bars === undefined || bars.length === 0 ? undefined : computeBarIndicators(bars);

          return toolResult({
            instrument: resolved,
            ticker: ticker ?? null,
            quote: quote ?? null,
            lastBars: bars?.slice(-5) ?? null,
            ...(includeBars ? { bars: bars ?? null } : {}),
            indicators: indicators === undefined ? null : summarizeIndicators(indicators),
            orderBook:
              orderBook === undefined
                ? null
                : {
                    bids: orderBook.bids.slice(0, 5),
                    asks: orderBook.asks.slice(0, 5),
                    timestampMs: orderBook.timestampMs,
                    sequence: orderBook.sequence,
                  },
          });
        },
      }),
    );
  }

  const fundamentalData = options.fundamentalData;
  if (fundamentalData !== undefined) {
    tools.push(
      createTool({
        name: 'finance_fundamental_analysis',
        description:
          '一次完成财务报表查询与基本面分析：财务比率、成长、DuPont、Piotroski；若可用则附带相对估值倍数。',
        inputSchema: object({
          instrument: INSTRUMENT_KEY_SCHEMA.describe('股票标的 key，例如 sec:AAPL'),
          periods: POSITIVE_INTEGER_SCHEMA.describe('获取报表期数，缺省 2（用于同比）').default(2),
          periodType: PERIOD_TYPE_SCHEMA.describe('报表期间类型，缺省 annual').default('annual'),
        }),
        execute: async ({ instrument, periods, periodType }, { signal }) => {
          const resolved = context.resolveInstrument(instrument);
          const [statements, ticker] = await Promise.all([
            fundamentalData.getFinancialStatements(resolved, {
              periods,
              periodType,
              signal,
            }),
            marketData?.getTicker === undefined
              ? Promise.resolve(undefined)
              : marketData.getTicker(resolved, signal),
          ]);
          const current = statements[0];
          if (current === undefined) {
            throw new FinanceToolError(`没有 ${instrument} 的财务报表`);
          }
          const prior = statements[1];
          const growth = prior === undefined ? undefined : computeGrowthMetrics(current, prior);
          const valuation =
            ticker === undefined
              ? undefined
              : computeValuationMultiples(current, {
                  price: ticker.lastPrice,
                  ...(growth?.revenueGrowth === undefined
                    ? {}
                    : { growthRate: growth.revenueGrowth }),
                });
          return toolResult({
            instrument: resolved,
            period: current.period,
            ratios: computeFundamentalRatios(current, prior),
            prior:
              prior === undefined
                ? null
                : {
                    period: prior.period,
                    ratios: computeFundamentalRatios(prior),
                    dupont: computeDuPont(prior),
                  },
            growth: growth ?? null,
            dupont: computeDuPont(current, prior),
            piotroski: prior === undefined ? null : computePiotroskiFScore(current, prior),
            valuation: valuation ?? null,
          });
        },
      }),
    );
  }

  const fxRates = options.fxRates;
  const fxBars = options.fxBars;
  if (fxRates !== undefined || fxBars !== undefined) {
    tools.push(
      createTool({
        name: 'finance_fx_analysis',
        description:
          '一次完成 FX 查询与分析：即期汇率 + 日线 + 技术指标摘要（参考汇率源的 OHLC 可能平坦）。',
        inputSchema: object({
          instrument: INSTRUMENT_KEY_SCHEMA.describe('FX 标的 key，例如 fx:EUR/USD'),
          lookbackDays: number({ isInteger: true, min: 5, max: 365 })
            .describe('回看自然日数，缺省 60')
            .default(DEFAULT_FX_LOOKBACK_DAYS),
        }),
        execute: async ({ instrument, lookbackDays }, { signal }) => {
          const resolved = context.resolveInstrument(instrument);
          if (resolved.assetClass !== 'fx') {
            throw new FinanceToolError(`标的 ${instrument} 不是 FX`);
          }
          const fx = resolved as FxInstrument;
          const toMs = nowMs();
          const [rate, bars] = await Promise.all([
            fxRates === undefined
              ? Promise.resolve(undefined)
              : fxRates.getRate(fx.baseCurrency, fx.quoteCurrency, signal),
            fxBars === undefined
              ? Promise.resolve(undefined)
              : fxBars.getBars(
                  fx,
                  {
                    interval: BarInterval.Day1,
                    fromMs: toMs - lookbackDays * 86_400_000,
                    toMs,
                    signal,
                  },
                  signal,
                ),
          ]);
          return toolResult({
            instrument: fx,
            rate: rate ?? null,
            lastBars: bars?.slice(-5) ?? null,
            indicators:
              bars === undefined || bars.length === 0
                ? null
                : summarizeIndicators(computeBarIndicators(bars)),
          });
        },
      }),
    );
  }

  const optionsProvider = options.options;
  if (optionsProvider !== undefined) {
    tools.push(
      createTool({
        name: 'finance_option_analysis',
        description:
          '一次完成期权查询与分析：选择到期日、拉取期权链、围绕 ATM 行权价返回精简合约摘要。',
        inputSchema: object({
          instrument: INSTRUMENT_KEY_SCHEMA.describe(
            '期权标的 key，例如 nasdaq:SPY 或 deribit:BTC',
          ),
          expiration: DATE_SCHEMA.describe('到期日 YYYY-MM-DD；缺省使用最近到期日').optional(),
          maxContracts: number({ isInteger: true, min: 2, max: 200 })
            .describe('最多返回的合约数，缺省 20')
            .default(DEFAULT_MAX_OPTION_CONTRACTS),
        }),
        execute: async ({ instrument, expiration, maxContracts }, { signal }) => {
          const resolved = context.resolveInstrument(instrument);
          const [expirations, ticker] = await Promise.all([
            optionsProvider.getExpirations(resolved, signal),
            marketData?.getTicker === undefined
              ? Promise.resolve(undefined)
              : marketData.getTicker(resolved, signal),
          ]);
          if (expirations.length === 0) {
            throw new FinanceToolError(`没有 ${instrument} 的期权到期日`);
          }
          const expirationMs =
            expiration === undefined ? (expirations[0] as number) : parseDateUtc(expiration);
          const chain = await optionsProvider.getChain(resolved, expirationMs, signal);
          const underlyingPrice = ticker?.lastPrice;
          const strikes = [
            ...new Map(
              chain.contracts.map((item) => [item.strike.toString(), item.strike]),
            ).values(),
          ];
          let atmStrike: Decimal | undefined;
          if (underlyingPrice === undefined) {
            atmStrike = strikes[Math.floor(strikes.length / 2)];
          } else {
            for (const strike of strikes) {
              if (
                atmStrike === undefined ||
                strike
                  .subtract(underlyingPrice)
                  .abs()
                  .compare(atmStrike.subtract(underlyingPrice).abs()) < 0
              ) {
                atmStrike = strike;
              }
            }
          }
          const selected = [...chain.contracts]
            .sort((left, right) => {
              if (atmStrike === undefined) return 0;
              return left.strike
                .subtract(atmStrike)
                .abs()
                .compare(right.strike.subtract(atmStrike).abs());
            })
            .slice(0, maxContracts)
            .map((contract) => ({
              contractId: contract.contractId,
              optionType: contract.optionType,
              strike: contract.strike,
              bid: contract.bid ?? null,
              ask: contract.ask ?? null,
              last: contract.last ?? null,
              volume: contract.volume ?? null,
              openInterest: contract.openInterest ?? null,
            }));
          return toolResult({
            instrument: resolved,
            expirationMs,
            expirationDate: new Date(expirationMs).toISOString().slice(0, 10),
            underlyingPrice: underlyingPrice ?? null,
            atmStrike: atmStrike ?? null,
            totalContracts: chain.contracts.length,
            contracts: selected,
          });
        },
      }),
    );
  }

  const yieldCurveProvider = options.yieldCurve;
  if (yieldCurveProvider !== undefined) {
    tools.push(
      createTool({
        name: 'finance_yield_curve',
        description: '查询收益率曲线；不传日期时取最新，返回期限点与 10Y 等关键期限。',
        inputSchema: object({
          currency: CURRENCY_CODE_SCHEMA.describe('曲线货币，缺省 USD').default('USD'),
          date: DATE_SCHEMA.describe('曲线日期 YYYY-MM-DD；缺省最新').optional(),
        }),
        execute: async ({ currency, date }, { signal }) => {
          const resolved = context.resolveCurrency(currency);
          const curve =
            date === undefined
              ? await yieldCurveProvider.getLatest(resolved, signal)
              : await yieldCurveProvider.getCurve(date, resolved, signal);
          const tenYear = curve.points.find((point) => point.tenorYears === 10);
          return toolResult({
            date: curve.date,
            currency: curve.currency,
            tenYear: tenYear ?? null,
            points: curve.points,
          });
        },
      }),
    );
  }

  return Object.freeze(tools);
}

/** 把指标序列压缩为最新值摘要。 */
function summarizeIndicators(series: BarIndicatorSeries): Record<string, unknown> {
  return {
    timestampMs: series.timestamps[series.timestamps.length - 1] ?? null,
    sma: mapLastValues(series.sma),
    ema: mapLastValues(series.ema),
    rsi: lastValue(series.rsi),
    macd:
      series.macd === null
        ? null
        : {
            macd: lastValue(series.macd.macd),
            signal: lastValue(series.macd.signal),
            histogram: lastValue(series.macd.histogram),
          },
    bollinger:
      series.bollinger === null
        ? null
        : {
            middle: lastValue(series.bollinger.middle),
            upper: lastValue(series.bollinger.upper),
            lower: lastValue(series.bollinger.lower),
            bandwidth: lastValue(series.bollinger.bandwidth),
            percentB: lastValue(series.bollinger.percentB),
          },
    atr: series.atr === null ? null : lastValue(series.atr),
    stochastic:
      series.stochastic === null
        ? null
        : {
            k: lastValue(series.stochastic.k),
            d: lastValue(series.stochastic.d),
          },
    vwap: series.vwap === null ? null : lastValue(series.vwap),
  };
}

/** 取数组最后一个非 null 值。 */
function lastValue(values: readonly (number | null)[]): number | null {
  for (let index = values.length - 1; index >= 0; index -= 1) {
    const value = values[index];
    if (value !== null && value !== undefined) return value;
  }
  return null;
}

/** 把周期映射压缩为最新值。 */
function mapLastValues(
  values: Readonly<Record<string, readonly (number | null)[]>>,
): Record<string, number | null> {
  const result: Record<string, number | null> = {};
  for (const [period, series] of Object.entries(values)) {
    result[period] = lastValue(series);
  }
  return result;
}
