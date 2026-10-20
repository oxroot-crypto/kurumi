/**
 * @file Binance 现货/期货只读参考数据提供方。
 * @description 将 exchangeInfo 映射为 `Instrument`；只包含公开静态规格，不包含账户信息。
 * @module binance/reference
 */

import {
  AssetClass,
  createInstrument,
  createMoney,
  instrumentKey,
  type Currency,
  type Instrument,
  type ListInstrumentsOptions,
  type ReferenceDataProvider,
} from '@kurumi/finance';
import { Decimal } from '@kurumi/math';

import {
  currencyFor,
  getBinanceJson,
  requireArray,
  requireRecord,
  resolveBinanceOptions,
  type BinanceProviderOptions,
  type ResolvedBinanceOptions,
} from './support.js';

/** 现货参考数据选项。 */
export interface BinanceReferenceOptions extends BinanceProviderOptions {
  /** 只保留这些计价货币；缺省全部。 */
  readonly quoteAssets?: readonly string[];
  /** 是否包含非 TRADING 标的，缺省 false。 */
  readonly includeNonTrading?: boolean;
}

/** 期货参考数据选项。 */
export interface BinanceFuturesReferenceOptions extends BinanceReferenceOptions {
  /** 只保留这些合约类型，缺省 PERPETUAL 与季度合约。 */
  readonly contractTypes?: readonly string[];
}

/**
 * 创建 Binance 现货参考数据提供方。
 *
 * @param options - 货币精度、quoteAssets 与 includeNonTrading
 * @returns 只读 `ReferenceDataProvider`
 */
export function createBinanceReferenceDataProvider(
  options: BinanceReferenceOptions = {},
): ReferenceDataProvider {
  const settings = resolveBinanceOptions(options);
  return Object.freeze({
    id: settings.id,
    listInstruments: async (listOptions?: ListInstrumentsOptions) => {
      const instruments = await fetchSpotInstruments(settings, options, listOptions?.signal);
      return Object.freeze(
        instruments.filter((instrument) => matchesListOptions(instrument, listOptions)),
      );
    },
    getInstrument: async (key: string, signal?: AbortSignal) => {
      const instruments = await fetchSpotInstruments(settings, options, signal);
      return instruments.find((instrument) => instrumentKey(instrument) === key);
    },
    listCurrencies: async (signal?: AbortSignal) => {
      const instruments = await fetchSpotInstruments(settings, options, signal);
      return Object.freeze(uniqueCurrencies(instruments));
    },
  });
}

/**
 * 创建 Binance 期货/永续参考数据提供方。
 *
 * @param options - 货币精度、quoteAssets、contractTypes 与 includeNonTrading
 * @returns 只读 `ReferenceDataProvider`
 */
export function createBinanceFuturesReferenceDataProvider(
  options: BinanceFuturesReferenceOptions = {},
): ReferenceDataProvider {
  const settings = resolveBinanceOptions(options);
  return Object.freeze({
    id: settings.id,
    listInstruments: async (listOptions?: ListInstrumentsOptions) => {
      const instruments = await fetchFuturesInstruments(settings, options, listOptions?.signal);
      return Object.freeze(
        instruments.filter((instrument) => matchesListOptions(instrument, listOptions)),
      );
    },
    getInstrument: async (key: string, signal?: AbortSignal) => {
      const instruments = await fetchFuturesInstruments(settings, options, signal);
      return instruments.find((instrument) => instrumentKey(instrument) === key);
    },
    listCurrencies: async (signal?: AbortSignal) => {
      const instruments = await fetchFuturesInstruments(settings, options, signal);
      return Object.freeze(uniqueCurrencies(instruments));
    },
  });
}

/** 拉取并映射现货标的。 */
async function fetchSpotInstruments(
  settings: ResolvedBinanceOptions,
  options: BinanceReferenceOptions,
  signal?: AbortSignal,
): Promise<readonly Instrument[]> {
  const payload = await getBinanceJson(
    settings,
    settings.baseUrl,
    '/api/v3/exchangeInfo',
    {},
    signal,
  );
  const root = requireRecord(payload, '$', settings.id);
  const symbols = requireArray(root['symbols'], '$.symbols', settings.id);
  const instruments: Instrument[] = [];
  for (let index = 0; index < symbols.length; index += 1) {
    const symbol = requireRecord(symbols[index], `$.symbols[${index}]`, settings.id);
    const instrument = mapSpotInstrument(settings, symbol, options);
    if (instrument !== undefined) instruments.push(instrument);
  }
  instruments.sort((left, right) => instrumentKey(left).localeCompare(instrumentKey(right)));
  return Object.freeze(instruments);
}

/** 拉取并映射期货标的。 */
async function fetchFuturesInstruments(
  settings: ResolvedBinanceOptions,
  options: BinanceFuturesReferenceOptions,
  signal?: AbortSignal,
): Promise<readonly Instrument[]> {
  const payload = await getBinanceJson(
    settings,
    settings.futuresBaseUrl,
    '/fapi/v1/exchangeInfo',
    {},
    signal,
  );
  const root = requireRecord(payload, '$', settings.id);
  const symbols = requireArray(root['symbols'], '$.symbols', settings.id);
  const instruments: Instrument[] = [];
  for (let index = 0; index < symbols.length; index += 1) {
    const symbol = requireRecord(symbols[index], `$.symbols[${index}]`, settings.id);
    const instrument = mapFuturesInstrument(settings, symbol, options);
    if (instrument !== undefined) instruments.push(instrument);
  }
  instruments.sort((left, right) => instrumentKey(left).localeCompare(instrumentKey(right)));
  return Object.freeze(instruments);
}

/** 映射现货 symbol。 */
function mapSpotInstrument(
  settings: ResolvedBinanceOptions,
  symbol: Record<string, unknown>,
  options: BinanceReferenceOptions,
): Instrument | undefined {
  const status = String(symbol['status'] ?? '');
  if (options.includeNonTrading !== true && status !== 'TRADING') return undefined;
  const baseAsset = String(symbol['baseAsset'] ?? '');
  const quoteAsset = String(symbol['quoteAsset'] ?? '');
  if (baseAsset === '' || quoteAsset === '') return undefined;
  if (options.quoteAssets !== undefined && !options.quoteAssets.includes(quoteAsset)) {
    return undefined;
  }
  const quoteCurrency = currencyFor(settings, quoteAsset);
  const tickSize = filterDecimal(symbol, 'PRICE_FILTER', 'tickSize', settings.id);
  const lotSize = filterDecimal(symbol, 'LOT_SIZE', 'stepSize', settings.id);
  if (tickSize === undefined || lotSize === undefined) return undefined;
  if (!tickSize.isPositive() || !lotSize.isPositive()) return undefined;
  const minQuantity = filterDecimal(symbol, 'LOT_SIZE', 'minQty', settings.id);
  const maxQuantity = filterDecimal(symbol, 'LOT_SIZE', 'maxQty', settings.id);
  const minNotional =
    filterDecimal(symbol, 'NOTIONAL', 'minNotional', settings.id) ??
    filterDecimal(symbol, 'MIN_NOTIONAL', 'minNotional', settings.id);

  try {
    return createInstrument({
      assetClass: AssetClass.Crypto,
      venue: settings.venue,
      symbol: `${baseAsset}/${quoteAsset}`,
      baseAsset,
      quoteCurrency,
      tickSize,
      lotSize,
      ...(minQuantity === undefined || !minQuantity.isPositive() ? {} : { minQuantity }),
      ...(maxQuantity === undefined || !maxQuantity.isPositive() ? {} : { maxQuantity }),
      ...(minNotional === undefined || !minNotional.isPositive()
        ? {}
        : { minNotional: createMoney(minNotional, quoteCurrency) }),
      isTradable: status === 'TRADING',
    });
  } catch {
    return undefined;
  }
}

/** 映射期货 symbol。 */
function mapFuturesInstrument(
  settings: ResolvedBinanceOptions,
  symbol: Record<string, unknown>,
  options: BinanceFuturesReferenceOptions,
): Instrument | undefined {
  const status = String(symbol['status'] ?? '');
  if (options.includeNonTrading !== true && status !== 'TRADING') return undefined;
  const contractType = String(symbol['contractType'] ?? '');
  if (options.contractTypes !== undefined && !options.contractTypes.includes(contractType)) {
    return undefined;
  }
  const baseAsset = String(symbol['baseAsset'] ?? '');
  const quoteAsset = String(symbol['quoteAsset'] ?? '');
  if (baseAsset === '' || quoteAsset === '') return undefined;
  if (options.quoteAssets !== undefined && !options.quoteAssets.includes(quoteAsset)) {
    return undefined;
  }
  const quoteCurrency = currencyFor(settings, quoteAsset);
  const tickSize = filterDecimal(symbol, 'PRICE_FILTER', 'tickSize', settings.id);
  const lotSize = filterDecimal(symbol, 'LOT_SIZE', 'stepSize', settings.id);
  if (tickSize === undefined || lotSize === undefined) return undefined;
  if (!tickSize.isPositive() || !lotSize.isPositive()) return undefined;
  const minQuantity = filterDecimal(symbol, 'LOT_SIZE', 'minQty', settings.id);
  const minNotional =
    filterDecimal(symbol, 'MIN_NOTIONAL', 'notional', settings.id) ??
    filterDecimal(symbol, 'NOTIONAL', 'minNotional', settings.id);
  const exchangeSymbol = String(symbol['symbol'] ?? '');
  const symbolName = exchangeSymbol === '' ? `${baseAsset}${quoteAsset}` : exchangeSymbol;
  const isPerpetual = contractType === 'PERPETUAL';
  const deliveryDate = Number(symbol['deliveryDate']);
  if (!isPerpetual && (!Number.isSafeInteger(deliveryDate) || deliveryDate <= 0)) {
    return undefined;
  }
  const fundingHours = Number(symbol['fundingIntervalHours'] ?? 8);
  const fundingIntervalMs =
    Number.isFinite(fundingHours) && fundingHours > 0
      ? Math.round(fundingHours * 3_600_000)
      : 28_800_000;

  try {
    if (isPerpetual) {
      return createInstrument({
        assetClass: AssetClass.Perpetual,
        venue: settings.futuresVenue,
        symbol: symbolName,
        quoteCurrency,
        tickSize,
        lotSize,
        ...(minQuantity === undefined || !minQuantity.isPositive() ? {} : { minQuantity }),
        ...(minNotional === undefined || !minNotional.isPositive()
          ? {}
          : { minNotional: createMoney(minNotional, quoteCurrency) }),
        fundingIntervalMs,
        isTradable: status === 'TRADING',
      });
    }
    return createInstrument({
      assetClass: AssetClass.Future,
      venue: settings.futuresVenue,
      symbol: symbolName,
      quoteCurrency,
      tickSize,
      lotSize,
      ...(minQuantity === undefined || !minQuantity.isPositive() ? {} : { minQuantity }),
      ...(minNotional === undefined || !minNotional.isPositive()
        ? {}
        : { minNotional: createMoney(minNotional, quoteCurrency) }),
      expiryMs: deliveryDate,
      settlement: 'cash',
      isTradable: status === 'TRADING',
    });
  } catch {
    return undefined;
  }
}

/** 读取 filter 中的 Decimal 字段。 */
function filterDecimal(
  symbol: Record<string, unknown>,
  filterType: string,
  field: string,
  providerId: string,
): Decimal | undefined {
  const filters = requireArray(symbol['filters'], `$.symbols.filters`, providerId);
  for (const filter of filters) {
    const record = requireRecord(filter, '$.symbols.filters[]', providerId);
    if (record['filterType'] !== filterType) continue;
    const value = record[field];
    if (typeof value !== 'string' || value === '') return undefined;
    try {
      return Decimal.parse(value);
    } catch {
      return undefined;
    }
  }
  return undefined;
}

/** 按 list 选项过滤。 */
function matchesListOptions(
  instrument: Instrument,
  options: { readonly venue?: string; readonly assetClass?: AssetClass } | undefined,
): boolean {
  if (options?.venue !== undefined && instrument.venue !== options.venue) return false;
  if (options?.assetClass !== undefined && instrument.assetClass !== options.assetClass) {
    return false;
  }
  return true;
}

/** 提取唯一货币。 */
function uniqueCurrencies(instruments: readonly Instrument[]): Currency[] {
  const currencies = new Map<string, Currency>();
  for (const instrument of instruments) {
    currencies.set(instrument.quoteCurrency.code, instrument.quoteCurrency);
  }
  return [...currencies.values()].sort((left, right) => left.code.localeCompare(right.code));
}
