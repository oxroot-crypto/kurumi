/**
 * @file Provider 查询类 Agent 工具。
 * @description 只依赖 finance provider 接口；具体 provider 由调用方注入，工具包不绑定任何数据源。
 * @module finance-tools/provider-tools
 */

import { createTool, type ToolDefinition } from '@kurumi/agent';
import { BarInterval, type FxInstrument, type ListInstrumentsOptions } from '@kurumi/finance';
import { number, object, string } from '@kurumi/schema';

import { FinanceToolError } from './errors.js';
import { toolResult } from './json.js';
import { createToolContext, type FinanceProviderToolOptions } from './options.js';
import {
  ASSET_CLASS_SCHEMA,
  CURRENCY_CODE_SCHEMA,
  DATE_SCHEMA,
  INSTRUMENT_KEY_SCHEMA,
  INTERVAL_SCHEMA,
  PERIOD_TYPE_SCHEMA,
  POSITIVE_INTEGER_SCHEMA,
  TIMESTAMP_MS_SCHEMA,
} from './schemas.js';

/**
 * 创建 provider 查询类工具。
 *
 * @param options - 注入的 provider、标的与货币配置
 * @returns 只读工具定义数组；未提供的能力不会生成工具
 */
export function createFinanceProviderTools(
  options: FinanceProviderToolOptions,
): readonly ToolDefinition[] {
  const context = createToolContext(options);
  const tools: ToolDefinition[] = [];

  const marketData = options.marketData;
  if (marketData?.getTicker !== undefined) {
    const getTicker = marketData.getTicker.bind(marketData);
    tools.push(
      createTool({
        name: 'finance_get_ticker',
        description: '查询标的的最新 Ticker 快照（只读）。',
        inputSchema: object({ instrument: INSTRUMENT_KEY_SCHEMA }),
        execute: async ({ instrument }, { signal }) =>
          toolResult(await getTicker(context.resolveInstrument(instrument), signal)),
      }),
    );
  }
  if (marketData?.getQuote !== undefined) {
    const getQuote = marketData.getQuote.bind(marketData);
    tools.push(
      createTool({
        name: 'finance_get_quote',
        description: '查询标的的买一/卖一报价（只读）。',
        inputSchema: object({ instrument: INSTRUMENT_KEY_SCHEMA }),
        execute: async ({ instrument }, { signal }) =>
          toolResult(await getQuote(context.resolveInstrument(instrument), signal)),
      }),
    );
  }
  if (marketData?.getOrderBook !== undefined) {
    const getOrderBook = marketData.getOrderBook.bind(marketData);
    tools.push(
      createTool({
        name: 'finance_get_order_book',
        description: '查询标的盘口快照，可指定档位深度（只读）。',
        inputSchema: object({
          instrument: INSTRUMENT_KEY_SCHEMA,
          depth: number({ isInteger: true, min: 1, max: 5_000 })
            .describe('盘口档位深度，1..5000；缺省由 provider 决定')
            .optional(),
        }),
        execute: async ({ instrument, depth }, { signal }) =>
          toolResult(
            await getOrderBook(context.resolveInstrument(instrument), {
              ...(depth === undefined ? {} : { depth }),
              signal,
            }),
          ),
      }),
    );
  }
  if (marketData?.getBars !== undefined) {
    const getBars = marketData.getBars.bind(marketData);
    tools.push(
      createTool({
        name: 'finance_get_bars',
        description: '只查询标的原始 K 线数据（只读）；需要指标请使用 finance_market_analysis。',
        inputSchema: object({
          instrument: INSTRUMENT_KEY_SCHEMA,
          interval: INTERVAL_SCHEMA,
          fromMs: TIMESTAMP_MS_SCHEMA.describe('查询起始时间（epoch 毫秒，含）'),
          toMs: TIMESTAMP_MS_SCHEMA.describe('查询结束时间（epoch 毫秒，不含）'),
        }),
        execute: async ({ instrument, interval, fromMs, toMs }, { signal }) =>
          toolResult(
            await getBars(context.resolveInstrument(instrument), {
              interval: interval as BarInterval,
              fromMs,
              toMs,
              signal,
            }),
          ),
      }),
    );
  }
  if (marketData?.getTrades !== undefined) {
    const getTrades = marketData.getTrades.bind(marketData);
    tools.push(
      createTool({
        name: 'finance_get_trades',
        description: '查询标的最近成交（只读）。',
        inputSchema: object({
          instrument: INSTRUMENT_KEY_SCHEMA,
          limit: number({ isInteger: true, min: 1, max: 1_000 })
            .describe('最多返回的成交条数，1..1000；缺省由 provider 决定')
            .optional(),
        }),
        execute: async ({ instrument, limit }, { signal }) =>
          toolResult(
            await getTrades(context.resolveInstrument(instrument), {
              ...(limit === undefined ? {} : { limit }),
              signal,
            }),
          ),
      }),
    );
  }

  const referenceData = options.referenceData;
  if (referenceData !== undefined) {
    tools.push(
      createTool({
        name: 'finance_list_instruments',
        description: '列出数据源支持的标的（只读）。',
        inputSchema: object({
          venue: string({ isNonEmpty: true })
            .describe('交易场所过滤，例如 binance、nasdaq；缺省不限制')
            .optional(),
          assetClass: ASSET_CLASS_SCHEMA.describe('资产类别过滤；缺省不限制').optional(),
        }),
        execute: async ({ venue, assetClass }, { signal }) => {
          const query: ListInstrumentsOptions = {
            ...(venue === undefined ? {} : { venue }),
            ...(assetClass === undefined ? {} : { assetClass }),
            signal,
          };
          return toolResult(await referenceData.listInstruments(query));
        },
      }),
      createTool({
        name: 'finance_get_instrument',
        description: '按 key 查询标的静态规格（只读）。',
        inputSchema: object({ key: INSTRUMENT_KEY_SCHEMA }),
        execute: async ({ key }, { signal }) =>
          toolResult(await referenceData.getInstrument(key, signal)),
      }),
    );
  }

  const fundamentalData = options.fundamentalData;
  if (fundamentalData?.getCompanyProfile !== undefined) {
    const getCompanyProfile = fundamentalData.getCompanyProfile.bind(fundamentalData);
    tools.push(
      createTool({
        name: 'finance_get_company_profile',
        description: '查询公司资料（只读）。',
        inputSchema: object({ instrument: INSTRUMENT_KEY_SCHEMA }),
        execute: async ({ instrument }, { signal }) =>
          toolResult(await getCompanyProfile(context.resolveInstrument(instrument), signal)),
      }),
    );
  }
  if (fundamentalData !== undefined) {
    tools.push(
      createTool({
        name: 'finance_get_financial_statements',
        description: '查询公司财务报表（只读，按报告期从新到旧）。',
        inputSchema: object({
          instrument: INSTRUMENT_KEY_SCHEMA,
          periods: POSITIVE_INTEGER_SCHEMA.describe(
            '最多返回的报表期数，缺省由 provider 决定',
          ).optional(),
          periodType: PERIOD_TYPE_SCHEMA.describe('报表期间类型，缺省 annual').optional(),
        }),
        execute: async ({ instrument, periods, periodType }, { signal }) =>
          toolResult(
            await fundamentalData.getFinancialStatements(context.resolveInstrument(instrument), {
              ...(periods === undefined ? {} : { periods }),
              ...(periodType === undefined ? {} : { periodType }),
              signal,
            }),
          ),
      }),
    );
  }

  const fxRates = options.fxRates;
  if (fxRates !== undefined) {
    tools.push(
      createTool({
        name: 'finance_get_fx_rate',
        description: '查询一组货币对的即期汇率（只读）。',
        inputSchema: object({
          base: CURRENCY_CODE_SCHEMA,
          quote: CURRENCY_CODE_SCHEMA,
        }),
        execute: async ({ base, quote }, { signal }) => {
          const rate = await fxRates.getRate(
            context.resolveCurrency(base),
            context.resolveCurrency(quote),
            signal,
          );
          if (rate === undefined) {
            throw new FinanceToolError(`没有 ${base}/${quote} 汇率`);
          }
          return toolResult(rate);
        },
      }),
      createTool({
        name: 'finance_list_fx_rates',
        description: '列出基准货币的全部可用汇率（只读）。',
        inputSchema: object({}),
        execute: async (_input, { signal }) => toolResult(await fxRates.listRates(signal)),
      }),
    );
  }

  const fxBars = options.fxBars;
  if (fxBars !== undefined) {
    tools.push(
      createTool({
        name: 'finance_get_fx_bars',
        description: '只查询 FX 原始日线（只读）；需要指标请使用 finance_fx_analysis。',
        inputSchema: object({
          instrument: INSTRUMENT_KEY_SCHEMA,
          interval: INTERVAL_SCHEMA,
          fromMs: TIMESTAMP_MS_SCHEMA.describe('查询起始时间（epoch 毫秒，含）'),
          toMs: TIMESTAMP_MS_SCHEMA.describe('查询结束时间（epoch 毫秒，不含）'),
        }),
        execute: async ({ instrument, interval, fromMs, toMs }, { signal }) => {
          const resolved = context.resolveInstrument(instrument);
          if (resolved.assetClass !== 'fx') {
            throw new FinanceToolError(`标的 ${instrument} 不是 FX`);
          }
          return toolResult(
            await fxBars.getBars(
              resolved as FxInstrument,
              {
                interval: interval as BarInterval,
                fromMs,
                toMs,
                signal,
              },
              signal,
            ),
          );
        },
      }),
    );
  }

  const optionsProvider = options.options;
  if (optionsProvider !== undefined) {
    tools.push(
      createTool({
        name: 'finance_get_option_expirations',
        description: '列出标的可用期权到期日（只读）。',
        inputSchema: object({ instrument: INSTRUMENT_KEY_SCHEMA }),
        execute: async ({ instrument }, { signal }) => {
          const expirations = await optionsProvider.getExpirations(
            context.resolveInstrument(instrument),
            signal,
          );
          return toolResult(
            expirations.map((expirationMs) => ({
              expirationMs,
              date: new Date(expirationMs).toISOString().slice(0, 10),
            })),
          );
        },
      }),
      createTool({
        name: 'finance_get_options_chain',
        description: '查询指定到期日的期权链（只读）。',
        inputSchema: object({
          instrument: INSTRUMENT_KEY_SCHEMA,
          expiration: DATE_SCHEMA,
        }),
        execute: async ({ instrument, expiration }, { signal }) => {
          const expirationMs = parseDateUtc(expiration);
          return toolResult(
            await optionsProvider.getChain(
              context.resolveInstrument(instrument),
              expirationMs,
              signal,
            ),
          );
        },
      }),
    );
  }

  const yieldCurve = options.yieldCurve;
  if (yieldCurve !== undefined) {
    tools.push(
      createTool({
        name: 'finance_get_yield_curve',
        description: '查询收益率曲线（只读）；不传日期时取最新。',
        inputSchema: object({
          currency: CURRENCY_CODE_SCHEMA.default('USD'),
          date: DATE_SCHEMA.optional(),
        }),
        execute: async ({ currency, date }, { signal }) => {
          const resolved = context.resolveCurrency(currency);
          const curve =
            date === undefined
              ? await yieldCurve.getLatest(resolved, signal)
              : await yieldCurve.getCurve(date, resolved, signal);
          return toolResult(curve);
        },
      }),
    );
  }

  return Object.freeze(tools);
}

/**
 * 解析 `YYYY-MM-DD` 为 UTC 毫秒。
 *
 * @param value - 日期
 * @returns UTC 毫秒
 */
export function parseDateUtc(value: string): number {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (match === null) throw new FinanceToolError(`日期格式非法: ${value}`);
  const year = Number(match[1]);
  const month = Number(match[2]) - 1;
  const day = Number(match[3]);
  const timestamp = Date.UTC(year, month, day);
  const parsed = new Date(timestamp);
  if (
    parsed.getUTCFullYear() !== year ||
    parsed.getUTCMonth() !== month ||
    parsed.getUTCDate() !== day
  ) {
    throw new FinanceToolError(`日期非法: ${value}`);
  }
  return timestamp;
}
