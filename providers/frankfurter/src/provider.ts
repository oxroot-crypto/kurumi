/**
 * @file Frankfurter（ECB）只读汇率提供方。
 * @description 使用 api.frankfurter.app 公开汇率接口；仅提供参考汇率，不用于交易执行。
 * @module frankfurter/provider
 */

import { isPlainObject } from '@kurumi/core';
import {
  BarInterval,
  ProviderDataError,
  UnsupportedProviderOperationError,
  createBar,
  createCurrency,
  createFxRate,
  fetchJson,
  isCurrency,
  roundPrice,
  type Bar,
  type Currency,
  type FetchLike,
  type FxInstrument,
  type FxRate,
  type FxRateProvider,
  type GetBarsOptions,
} from '@kurumi/finance';
import { Decimal } from '@kurumi/math';

/** Frankfurter 提供方选项。 */
export interface FrankfurterProviderOptions {
  /** 提供方 id，缺省 `frankfurter`。 */
  readonly id?: string;
  /** API 根地址，缺省 `https://api.frankfurter.app`。 */
  readonly baseUrl?: string;
  /** 可注入 fetch。 */
  readonly fetchImpl?: FetchLike;
  /** 额外请求头。 */
  readonly headers?: Readonly<Record<string, string>>;
  /** 超时毫秒，缺省 15_000。 */
  readonly timeoutMs?: number;
  /** 基准货币，缺省 EUR。 */
  readonly baseCurrency?: Currency;
  /** 货币精度覆盖。 */
  readonly currencyDecimals?: Readonly<Record<string, number>>;
}

/** Frankfurter 提供方（含货币列表扩展）。 */
export interface FrankfurterProvider extends FxRateProvider {
  /**
   * 列出 ECB 支持的货币。
   *
   * @param signal - 取消信号
   */
  listCurrencies(signal?: AbortSignal): Promise<readonly Currency[]>;
  /**
   * 获取 FX 日线。Frankfurter 只提供 ECB 参考汇率，因此 OHLC 相同、成交量为 0。
   *
   * @param instrument - FX 标的
   * @param options - 周期（仅 1d）与时间范围
   * @param signal - 取消信号
   */
  getBars(
    instrument: FxInstrument,
    options: GetBarsOptions,
    signal?: AbortSignal,
  ): Promise<readonly Bar[]>;
}

/** 解析后设置。 */
interface ResolvedFrankfurterOptions {
  readonly id: string;
  readonly baseUrl: string;
  readonly fetchImpl: FetchLike | undefined;
  readonly headers: Readonly<Record<string, string>>;
  readonly timeoutMs: number;
  readonly baseCurrency: Currency;
  readonly currencyDecimals: Readonly<Record<string, number>>;
}

/** 常用货币精度。 */
const DEFAULT_DECIMALS: Readonly<Record<string, number>> = Object.freeze({
  JPY: 0,
  KRW: 0,
  CLP: 0,
  ISK: 0,
  VND: 0,
  HUF: 2,
});

/**
 * 创建 Frankfurter 汇率提供方。
 *
 * @param options - base URL、fetch、基准货币与货币精度
 * @returns 只读 `FrankfurterProvider`
 *
 * @example
 * ```ts
 * const provider = createFrankfurterProvider();
 * const rate = await provider.getRate(eur, usd);
 * ```
 */
export function createFrankfurterProvider(
  options: FrankfurterProviderOptions = {},
): FrankfurterProvider {
  const settings = resolveOptions(options);
  return Object.freeze({
    id: settings.id,
    getRate: (base: Currency, quote: Currency, signal?: AbortSignal) =>
      getRate(settings, base, quote, signal),
    listRates: (signal?: AbortSignal) => listRates(settings, signal),
    listCurrencies: (signal?: AbortSignal) => listCurrencies(settings, signal),
    getBars: (instrument: FxInstrument, options: GetBarsOptions, signal?: AbortSignal) =>
      getBars(settings, instrument, options, signal),
  });
}

/** 查询单条汇率。 */
async function getRate(
  settings: ResolvedFrankfurterOptions,
  base: Currency,
  quote: Currency,
  signal?: AbortSignal,
): Promise<FxRate> {
  assertCurrency(settings.id, base, 'base');
  assertCurrency(settings.id, quote, 'quote');
  if (base.code === quote.code) {
    throw new ProviderDataError(settings.id, 'base 与 quote 不能相同');
  }
  const payload = await request(settings, '/latest', { from: base.code, to: quote.code }, signal);
  const record = requireRecord(payload, '$', settings.id);
  const rates = requireRecord(record['rates'], '$.rates', settings.id);
  const value = rates[quote.code];
  if (value === undefined) {
    throw new ProviderDataError(settings.id, `响应缺少 ${quote.code} 汇率`);
  }
  return createFxRate({
    base,
    quote,
    rate: parseRate(value, quote.code, settings.id),
    timestampMs: parseDate(record['date'], settings.id),
  });
}

/** 查询基准货币的全部汇率。 */
async function listRates(
  settings: ResolvedFrankfurterOptions,
  signal?: AbortSignal,
): Promise<readonly FxRate[]> {
  const payload = await request(settings, '/latest', { from: settings.baseCurrency.code }, signal);
  const record = requireRecord(payload, '$', settings.id);
  const baseCode = typeof record['base'] === 'string' ? record['base'] : settings.baseCurrency.code;
  const base = currencyFor(settings, baseCode);
  const rates = requireRecord(record['rates'], '$.rates', settings.id);
  const timestampMs = parseDate(record['date'], settings.id);
  return Object.freeze(
    Object.entries(rates)
      .filter(([code]) => code !== base.code)
      .map(([code, value]) =>
        createFxRate({
          base,
          quote: currencyFor(settings, code),
          rate: parseRate(value, code, settings.id),
          timestampMs,
        }),
      )
      .sort((left, right) => left.quote.code.localeCompare(right.quote.code)),
  );
}

/** 查询货币列表。 */
async function listCurrencies(
  settings: ResolvedFrankfurterOptions,
  signal?: AbortSignal,
): Promise<readonly Currency[]> {
  const payload = await request(settings, '/currencies', {}, signal);
  const record = requireRecord(payload, '$', settings.id);
  return Object.freeze(
    Object.keys(record)
      .map((code) => currencyFor(settings, code))
      .sort((left, right) => left.code.localeCompare(right.code)),
  );
}

/** 获取 FX 日线。 */
async function getBars(
  settings: ResolvedFrankfurterOptions,
  instrument: FxInstrument,
  options: GetBarsOptions,
  signal?: AbortSignal,
): Promise<readonly Bar[]> {
  if (options.interval !== BarInterval.Day1) {
    throw new UnsupportedProviderOperationError(settings.id, `getBars(${options.interval})`);
  }
  if (instrument.assetClass !== 'fx') {
    throw new ProviderDataError(settings.id, 'getBars 只接受 FX 标的');
  }
  if (
    !Number.isSafeInteger(options.fromMs) ||
    !Number.isSafeInteger(options.toMs) ||
    options.fromMs < 0 ||
    options.toMs <= options.fromMs
  ) {
    throw new ProviderDataError(settings.id, 'fromMs/toMs 必须是有效时间范围');
  }
  const start = formatDate(options.fromMs);
  // API 的结束日期是包含的；合约要求 toMs 不含，因此回退 1ms。
  const end = formatDate(options.toMs - 1);
  const payload = await request(
    settings,
    `/${start}..${end}`,
    { from: instrument.baseCurrency.code, to: instrument.quoteCurrency.code },
    signal ?? options.signal,
  );
  const record = requireRecord(payload, '$', settings.id);
  const rates = requireRecord(record['rates'], '$.rates', settings.id);
  const bars: Bar[] = [];
  for (const [date, value] of Object.entries(rates)) {
    const day = requireRecord(value, `$.rates.${date}`, settings.id);
    const raw = day[instrument.quoteCurrency.code];
    if (raw === undefined) continue;
    const rate = roundPrice(
      instrument,
      parseRate(raw, instrument.quoteCurrency.code, settings.id),
      'half-even',
    );
    const openTimeMs = parseDate(date, settings.id);
    if (openTimeMs < options.fromMs || openTimeMs >= options.toMs) continue;
    bars.push(
      createBar({
        instrument,
        interval: BarInterval.Day1,
        openTimeMs,
        closeTimeMs: openTimeMs + 86_400_000,
        open: rate,
        high: rate,
        low: rate,
        close: rate,
        volume: Decimal.ZERO,
      }),
    );
  }
  bars.sort((left, right) => left.openTimeMs - right.openTimeMs);
  return Object.freeze(bars);
}

/** 把毫秒时间格式化为 YYYY-MM-DD。 */
function formatDate(timestampMs: number): string {
  return new Date(timestampMs).toISOString().slice(0, 10);
}

/** 解析选项。 */
function resolveOptions(options: FrankfurterProviderOptions): ResolvedFrankfurterOptions {
  const id = options.id ?? 'frankfurter';
  if (id.trim() === '') throw new ProviderDataError('frankfurter', 'id 不能为空');
  const baseUrl = options.baseUrl ?? 'https://api.frankfurter.app';
  if (!URL.canParse(baseUrl)) throw new ProviderDataError(id, `baseUrl 非法: ${baseUrl}`);
  const baseCurrency = options.baseCurrency ?? createCurrency('EUR', 2);
  assertCurrency(id, baseCurrency, 'baseCurrency');
  return Object.freeze({
    id,
    baseUrl,
    fetchImpl: options.fetchImpl,
    headers: Object.freeze({ 'user-agent': 'kurumi-finance-provider/0.1', ...options.headers }),
    timeoutMs: options.timeoutMs ?? 15_000,
    baseCurrency,
    currencyDecimals: Object.freeze({ ...DEFAULT_DECIMALS, ...options.currencyDecimals }),
  });
}

/** 请求 JSON。 */
async function request(
  settings: ResolvedFrankfurterOptions,
  path: string,
  query: Readonly<Record<string, string>>,
  signal?: AbortSignal,
): Promise<unknown> {
  const url = new URL(path, settings.baseUrl);
  for (const [key, value] of Object.entries(query)) url.searchParams.set(key, value);
  return fetchJson(url.toString(), {
    providerId: settings.id,
    ...(settings.fetchImpl === undefined ? {} : { fetchImpl: settings.fetchImpl }),
    headers: settings.headers,
    timeoutMs: settings.timeoutMs,
    ...(signal === undefined ? {} : { signal }),
  });
}

/** 校验货币。 */
function assertCurrency(
  providerId: string,
  value: unknown,
  path: string,
): asserts value is Currency {
  if (!isCurrency(value)) {
    throw new ProviderDataError(providerId, `${path} 必须是货币`);
  }
}

/** 创建响应中的货币。 */
function currencyFor(settings: ResolvedFrankfurterOptions, code: string): Currency {
  try {
    return createCurrency(code, settings.currencyDecimals[code] ?? 2);
  } catch (cause) {
    throw new ProviderDataError(settings.id, `无法识别货币 ${code}`, cause);
  }
}

/** 解析汇率。 */
function parseRate(value: unknown, code: string, providerId: string): Decimal {
  try {
    const rate = Decimal.parse(String(value));
    if (!rate.isPositive()) throw new Error('汇率必须大于 0');
    return rate;
  } catch (cause) {
    throw new ProviderDataError(providerId, `${code} 汇率非法: ${String(value)}`, cause);
  }
}

/** 解析 YYYY-MM-DD。 */
function parseDate(value: unknown, providerId: string): number {
  if (typeof value !== 'string') {
    throw new ProviderDataError(providerId, '响应缺少 date');
  }
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (match === null) {
    throw new ProviderDataError(providerId, `date 格式非法: ${value}`);
  }
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
    throw new ProviderDataError(providerId, `date 非法: ${value}`);
  }
  return timestamp;
}

/** 校验普通对象。 */
function requireRecord(value: unknown, path: string, providerId: string): Record<string, unknown> {
  if (!isPlainObject(value)) {
    throw new ProviderDataError(providerId, `${path} 必须是对象`);
  }
  return value;
}
