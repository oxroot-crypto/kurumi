/**
 * @file Nasdaq 只读行情提供方：股票 Ticker、Quote 与日线。
 * @description 使用 api.nasdaq.com 公开接口；不提供盘口/逐笔，调用方应先检查方法是否存在。
 * @module nasdaq/market-data
 */

import {
  BarInterval,
  ProviderDataError,
  UnsupportedProviderOperationError,
  createBar,
  createQuote,
  createTicker,
  roundPrice,
  type Bar,
  type GetBarsOptions,
  type Instrument,
  type MarketDataProvider,
  type Quote,
  type Ticker,
} from '@kurumi/finance';
import { Decimal } from '@kurumi/math';

import {
  formatDate,
  getNasdaqJson,
  parseDecimalField,
  parseFlexibleTimestamp,
  parseSlashDate,
  requireArray,
  requireDecimalField,
  requireRecord,
  resolveNasdaqOptions,
  type NasdaqProviderOptions,
  type ResolvedNasdaqOptions,
} from './support.js';

/**
 * 创建 Nasdaq 行情提供方。
 *
 * @param options - base URL、fetch、货币、tickSize 与资产类别
 * @returns 只读 `MarketDataProvider`（实现 getTicker/getQuote/getBars）
 *
 * @example
 * ```ts
 * const provider = createNasdaqMarketDataProvider();
 * const ticker = await provider.getTicker!(aapl);
 * ```
 */
export function createNasdaqMarketDataProvider(
  options: NasdaqProviderOptions = {},
): MarketDataProvider {
  const settings = resolveNasdaqOptions(options);
  return Object.freeze({
    id: settings.id,
    getTicker: (instrument: Instrument, signal?: AbortSignal) =>
      getTicker(settings, instrument, signal),
    getQuote: (instrument: Instrument, signal?: AbortSignal) =>
      getQuote(settings, instrument, signal),
    getBars: (instrument: Instrument, barsOptions: GetBarsOptions) =>
      getBars(settings, instrument, barsOptions),
  });
}

/** 获取 Ticker。 */
async function getTicker(
  settings: ResolvedNasdaqOptions,
  instrument: Instrument,
  signal?: AbortSignal,
): Promise<Ticker> {
  const payload = await getNasdaqJson(
    settings,
    `/api/quote/${encodeURIComponent(instrument.symbol)}/info`,
    { assetclass: settings.assetClass },
    signal,
  );
  const root = requireRecord(payload, '$', settings.id);
  const data = requireRecord(root['data'], '$.data', settings.id);
  const primary = requireRecord(data['primaryData'], '$.data.primaryData', settings.id);
  const lastPrice = roundPrice(
    instrument,
    requireDecimalField(primary['lastSalePrice'], '$.data.primaryData.lastSalePrice', settings.id),
    'half-even',
  );
  const now = settings.nowMs();
  const timestampMs = parseFlexibleTimestamp(primary['lastTradeTimestamp'], now, settings.id);
  const bid = roundOptionalPrice(
    settings,
    instrument,
    primary['bidPrice'],
    '$.data.primaryData.bidPrice',
  );
  const ask = roundOptionalPrice(
    settings,
    instrument,
    primary['askPrice'],
    '$.data.primaryData.askPrice',
  );
  const volume = parseDecimalField(primary['volume'], '$.data.primaryData.volume', settings.id);

  return createTicker({
    instrument,
    lastPrice,
    ...(bid === undefined ? {} : { bid }),
    ...(ask === undefined ? {} : { ask }),
    ...(volume === undefined ? {} : { volume24h: volume }),
    timestampMs,
  });
}

/** 获取买一/卖一报价。 */
async function getQuote(
  settings: ResolvedNasdaqOptions,
  instrument: Instrument,
  signal?: AbortSignal,
): Promise<Quote> {
  const payload = await getNasdaqJson(
    settings,
    `/api/quote/${encodeURIComponent(instrument.symbol)}/info`,
    { assetclass: settings.assetClass },
    signal,
  );
  const root = requireRecord(payload, '$', settings.id);
  const data = requireRecord(root['data'], '$.data', settings.id);
  const primary = requireRecord(data['primaryData'], '$.data.primaryData', settings.id);
  const bid = roundPrice(
    instrument,
    requireDecimalField(primary['bidPrice'], '$.data.primaryData.bidPrice', settings.id),
    'half-even',
  );
  const ask = roundPrice(
    instrument,
    requireDecimalField(primary['askPrice'], '$.data.primaryData.askPrice', settings.id),
    'half-even',
  );
  const bidSize =
    parseDecimalField(primary['bidSize'], '$.data.primaryData.bidSize', settings.id) ??
    Decimal.ZERO;
  const askSize =
    parseDecimalField(primary['askSize'], '$.data.primaryData.askSize', settings.id) ??
    Decimal.ZERO;
  const timestampMs = parseFlexibleTimestamp(
    primary['lastTradeTimestamp'],
    settings.nowMs(),
    settings.id,
  );

  return createQuote({ instrument, bid, ask, bidSize, askSize, timestampMs });
}

/** 解析并按 tickSize 对齐可选价格。 */
function roundOptionalPrice(
  settings: ResolvedNasdaqOptions,
  instrument: Instrument,
  value: unknown,
  path: string,
): Decimal | undefined {
  const parsed = parseDecimalField(value, path, settings.id);
  return parsed === undefined ? undefined : roundPrice(instrument, parsed, 'half-even');
}

/** 获取日线。 */
async function getBars(
  settings: ResolvedNasdaqOptions,
  instrument: Instrument,
  options: GetBarsOptions,
): Promise<readonly Bar[]> {
  if (options.interval !== BarInterval.Day1) {
    throw new UnsupportedProviderOperationError(settings.id, `getBars(${options.interval})`);
  }
  if (!Number.isSafeInteger(options.fromMs) || !Number.isSafeInteger(options.toMs)) {
    throw new ProviderDataError(settings.id, 'fromMs/toMs 必须是安全整数');
  }
  if (options.toMs <= options.fromMs) {
    throw new ProviderDataError(settings.id, 'toMs 必须晚于 fromMs');
  }

  const payload = await getNasdaqJson(
    settings,
    `/api/quote/${encodeURIComponent(instrument.symbol)}/historical`,
    {
      assetclass: settings.assetClass,
      fromdate: formatDate(options.fromMs),
      todate: formatDate(options.toMs - 1),
      limit: String(settings.maxRows),
    },
    options.signal,
  );
  const root = requireRecord(payload, '$', settings.id);
  const data = requireRecord(root['data'], '$.data', settings.id);
  const tableValue = data['tradesTable'];
  if (tableValue === null || tableValue === undefined) return Object.freeze([]);
  const table = requireRecord(tableValue, '$.data.tradesTable', settings.id);
  const rows = requireArray(table['rows'], '$.data.tradesTable.rows', settings.id);
  const bars: Bar[] = [];

  for (let index = 0; index < rows.length; index += 1) {
    const row = requireRecord(rows[index], `$.data.tradesTable.rows[${index}]`, settings.id);
    const openTimeMs = parseSlashDate(
      row['date'],
      `$.data.tradesTable.rows[${index}].date`,
      settings.id,
    );
    const open = parseDecimalField(row['open'], `rows[${index}].open`, settings.id);
    const high = parseDecimalField(row['high'], `rows[${index}].high`, settings.id);
    const low = parseDecimalField(row['low'], `rows[${index}].low`, settings.id);
    const close = parseDecimalField(row['close'], `rows[${index}].close`, settings.id);
    const volume =
      parseDecimalField(row['volume'], `rows[${index}].volume`, settings.id) ?? Decimal.ZERO;
    if (open === undefined || high === undefined || low === undefined || close === undefined) {
      continue;
    }
    if (openTimeMs < options.fromMs || openTimeMs >= options.toMs) continue;
    // 合并行情可能带子最小变动价位；按标的 tick 对齐，保证 Bar 约束成立。
    const roundedOpen = roundPrice(instrument, open, 'half-even');
    const roundedHigh = roundPrice(instrument, high, 'half-even');
    const roundedLow = roundPrice(instrument, low, 'half-even');
    const roundedClose = roundPrice(instrument, close, 'half-even');
    bars.push(
      createBar({
        instrument,
        interval: BarInterval.Day1,
        openTimeMs,
        closeTimeMs: openTimeMs + 86_400_000,
        open: roundedOpen,
        high: roundedHigh,
        low: roundedLow,
        close: roundedClose,
        volume,
      }),
    );
  }

  bars.sort((left, right) => left.openTimeMs - right.openTimeMs);
  return Object.freeze(bars);
}
