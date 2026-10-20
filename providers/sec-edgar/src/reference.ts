/**
 * @file SEC EDGAR 只读参考数据提供方。
 * @description 使用 company_tickers.json 提供美股标的与货币列表；不包含价格行情。
 * @module sec-edgar/reference
 */

import {
  instrumentKey,
  type Instrument,
  type ListInstrumentsOptions,
  type ReferenceDataProvider,
} from '@kurumi/finance';

import {
  fetchTickers,
  instrumentForTicker,
  resolveSecOptions,
  type ResolvedSecOptions,
  type SecCompanyTicker,
  type SecEdgarProviderOptions,
} from './support.js';

/** ticker 缓存时长（毫秒）。 */
const TICKER_CACHE_TTL_MS = 60_000;

/**
 * 创建 SEC EDGAR 参考数据提供方。
 *
 * @param options - User-Agent、ticker URL 与 venue
 * @returns 只读 `ReferenceDataProvider`
 */
export function createSecEdgarReferenceProvider(
  options: SecEdgarProviderOptions,
): ReferenceDataProvider {
  const settings = resolveSecOptions(options);
  let cache:
    { readonly expiresAtMs: number; readonly tickers: readonly SecCompanyTicker[] } | undefined;

  const loadTickers = async (signal?: AbortSignal): Promise<readonly SecCompanyTicker[]> => {
    const now = settings.nowMs();
    if (cache !== undefined && cache.expiresAtMs > now) return cache.tickers;
    const tickers = await fetchTickers(settings, signal);
    cache = { expiresAtMs: now + TICKER_CACHE_TTL_MS, tickers };
    return tickers;
  };

  return Object.freeze({
    id: settings.id,
    listInstruments: async (listOptions?: ListInstrumentsOptions) => {
      const tickers = await loadTickers(listOptions?.signal);
      const instruments = tickers.map((ticker) => instrumentForTicker(settings, ticker));
      return Object.freeze(
        instruments.filter((instrument) => matchesOptions(settings, instrument, listOptions)),
      );
    },
    getInstrument: async (key: string, signal?: AbortSignal) => {
      const tickers = await loadTickers(signal);
      return tickers
        .map((ticker) => instrumentForTicker(settings, ticker))
        .find((instrument) => instrumentKey(instrument) === key);
    },
    listCurrencies: async () => Object.freeze([settings.currency]),
  });
}

/** 按查询选项过滤。 */
function matchesOptions(
  settings: ResolvedSecOptions,
  instrument: Instrument,
  options: ListInstrumentsOptions | undefined,
): boolean {
  if (options?.venue !== undefined && instrument.venue !== options.venue) return false;
  if (options?.assetClass !== undefined && instrument.assetClass !== options.assetClass) {
    return false;
  }
  return instrument.venue === settings.venue || options?.venue !== undefined;
}
