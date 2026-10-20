/**
 * @file 只读数据提供方契约桶导出。
 * @description 按领域拆分行情、参考数据、基本面、FX、期权、债券/收益率曲线；不提供任何交易执行接口。
 * @module finance/provider
 */

export type { FundamentalDataProvider, GetFinancialStatementsOptions } from './fundamentals.js';
export type {
  BondDataProvider,
  ListBondsOptions,
  YieldCurve,
  YieldCurvePoint,
  YieldCurveProvider,
} from './fixed-income.js';
export type { FxBarProvider, FxRateProvider } from './fx.js';
export type {
  GetBarsOptions,
  GetOrderBookOptions,
  GetTradesOptions,
  MarketDataProvider,
} from './market-data.js';
export type { OptionContractQuote, OptionsChain, OptionsDataProvider } from './options.js';
export type { ListInstrumentsOptions, ReferenceDataProvider } from './reference-data.js';
export {
  ProviderDataError,
  ProviderError,
  ProviderHttpError,
  UnsupportedProviderOperationError,
} from './errors.js';
export { fetchJson, fetchText, type FetchLike, type FetchJsonOptions } from './http.js';
