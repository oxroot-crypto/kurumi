/**
 * @file `@kurumi/binance` 主入口。
 * @description Binance 现货/期货公开数据只读适配器；不包含账户、下单或资金划转。
 * @module binance
 */

export { createBinanceFuturesMarketDataProvider } from './futures.js';
export {
  createBinanceFuturesReferenceDataProvider,
  createBinanceReferenceDataProvider,
  type BinanceFuturesReferenceOptions,
  type BinanceReferenceOptions,
} from './reference.js';
export { createBinanceMarketDataProvider } from './spot.js';
export type { BinanceProviderOptions } from './support.js';
