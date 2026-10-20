/**
 * @file 工具选项、标的/货币解析与共享 schema。
 * @description 工具包不导入任何 provider 包；provider 由调用方通过接口注入。
 * @module finance-tools/options
 */

import {
  createCurrency,
  createInstrumentRegistry,
  type AssetClass,
  type Instrument,
  type InstrumentRegistry,
  type FundamentalDataProvider,
  type FxBarProvider,
  type FxRateProvider,
  type MarketDataProvider,
  type OptionsDataProvider,
  type ReferenceDataProvider,
  type YieldCurveProvider,
  type Currency,
} from '@kurumi/finance';
import { FinanceToolError } from './errors.js';

/** 工具共享选项。 */
export interface FinanceToolOptions {
  /** 可解析的标的列表；与 resolveInstrument 至少提供一个。 */
  readonly instruments?: readonly Instrument[];
  /** 自定义标的解析器；优先于 instruments。 */
  readonly resolveInstrument?: (key: string) => Instrument | undefined;
  /** 货币精度覆盖。 */
  readonly currencies?: Readonly<Record<string, number>>;
  /** 当前时间注入；缺省 Date.now。 */
  readonly nowMs?: () => number;
}

/** provider 查询工具选项；不指定任何具体 provider。 */
export interface FinanceProviderToolOptions extends FinanceToolOptions {
  /** 行情提供方。 */
  readonly marketData?: MarketDataProvider;
  /** 参考数据提供方。 */
  readonly referenceData?: ReferenceDataProvider;
  /** 基本面提供方。 */
  readonly fundamentalData?: FundamentalDataProvider;
  /** FX 汇率提供方。 */
  readonly fxRates?: FxRateProvider;
  /** FX K 线提供方。 */
  readonly fxBars?: FxBarProvider;
  /** 期权链提供方。 */
  readonly options?: OptionsDataProvider;
  /** 收益率曲线提供方。 */
  readonly yieldCurve?: YieldCurveProvider;
}

/** 全部金融工具预设选项。 */
export interface FinanceToolsOptions extends FinanceProviderToolOptions {
  /** 是否包含纯分析工具，缺省 true。 */
  readonly shouldIncludeAnalysisTools?: boolean;
  /** 是否包含复合分析工具，缺省 true。 */
  readonly shouldIncludeCompositeTools?: boolean;
  /** 是否包含低层 provider 查询工具，缺省 true；无对应 provider 时自动跳过。 */
  readonly shouldIncludeProviderTools?: boolean;
}

/** 默认货币精度。 */
const DEFAULT_CURRENCY_DECIMALS: Readonly<Record<string, number>> = Object.freeze({
  USD: 2,
  EUR: 2,
  GBP: 2,
  JPY: 0,
  CNY: 2,
  HKD: 2,
  CHF: 2,
  AUD: 2,
  CAD: 2,
  USDT: 8,
  USDC: 8,
  BTC: 8,
  ETH: 8,
});

/** 已解析的工具上下文。 */
export interface FinanceToolContext {
  readonly registry: InstrumentRegistry | undefined;
  readonly resolveInstrument: (key: string) => Instrument;
  readonly resolveCurrency: (code: string) => Currency;
  readonly options: FinanceToolOptions;
}

/**
 * 解析工具上下文。
 *
 * @param options - 工具选项
 * @returns 标的/货币解析器
 */
export function createToolContext(options: FinanceToolOptions): FinanceToolContext {
  const registry =
    options.instruments === undefined ? undefined : createInstrumentRegistry(options.instruments);
  const customResolver = options.resolveInstrument;
  const currencyDecimals = Object.freeze({
    ...DEFAULT_CURRENCY_DECIMALS,
    ...options.currencies,
  });

  return Object.freeze({
    registry,
    resolveInstrument: (key: string): Instrument => {
      const custom = customResolver?.(key);
      if (custom !== undefined) return custom;
      const fromRegistry = registry?.get(key);
      if (fromRegistry !== undefined) return fromRegistry;
      throw new FinanceToolError(
        `未找到标的 "${key}"；请在 createFinanceTools options 中配置 instruments 或 resolveInstrument`,
      );
    },
    resolveCurrency: (code: string): Currency => {
      try {
        return createCurrency(code, currencyDecimals[code] ?? 2);
      } catch (cause) {
        throw new FinanceToolError(`无法识别货币 "${code}"`, cause);
      }
    },
    options,
  });
}

/** 判断资产类别是否为 FX。 */
export function isFxAssetClass(assetClass: AssetClass): boolean {
  return assetClass === 'fx';
}
