/**
 * @file SEC EDGAR 提供方共享选项、ticker 索引与标的映射。
 * @description SEC 要求带联系方式的 User-Agent；本模块统一校验并复用 ticker 索引。
 * @module sec-edgar/support
 */

import { isPlainObject } from '@kurumi/core';
import {
  AssetClass,
  ProviderDataError,
  createCurrency,
  createInstrument,
  fetchJson,
  instrumentKey,
  type Currency,
  type FetchLike,
  type Instrument,
} from '@kurumi/finance';
import { Decimal } from '@kurumi/math';

/** SEC 公司 ticker。 */
export interface SecCompanyTicker {
  /** 10 位 CIK。 */
  readonly cik: string;
  /** 股票代码。 */
  readonly ticker: string;
  /** 公司名称。 */
  readonly title: string;
}

/** SEC EDGAR 选项。 */
export interface SecEdgarProviderOptions {
  /** 提供方 id，缺省 `sec-edgar`。 */
  readonly id?: string;
  /** 数据 API 根地址，缺省 `https://data.sec.gov`。 */
  readonly baseUrl?: string;
  /** ticker 列表地址，缺省 `https://www.sec.gov/files/company_tickers.json`。 */
  readonly tickersUrl?: string;
  /** 可注入 fetch。 */
  readonly fetchImpl?: FetchLike;
  /** 额外请求头。 */
  readonly headers?: Readonly<Record<string, string>>;
  /** 超时毫秒，缺省 20_000。 */
  readonly timeoutMs?: number;
  /** 当前时间注入；参考数据缓存使用。 */
  readonly nowMs?: () => number;
  /** SEC 要求的 User-Agent，必须包含联系方式。 */
  readonly userAgent: string;
  /** 股票计价货币，缺省 USD。 */
  readonly currency?: Currency;
  /** 参考数据 venue，缺省 `sec`。 */
  readonly venue?: string;
  /** instrument key 或 ticker 到 CIK 的显式映射。 */
  readonly cikMap?: Readonly<Record<string, string>>;
}

/** 解析后的设置。 */
export interface ResolvedSecOptions {
  readonly id: string;
  readonly baseUrl: string;
  readonly tickersUrl: string;
  readonly fetchImpl: FetchLike | undefined;
  readonly headers: Readonly<Record<string, string>>;
  readonly timeoutMs: number;
  readonly nowMs: () => number;
  readonly currency: Currency;
  readonly venue: string;
  readonly cikMap: Readonly<Record<string, string>>;
}

/**
 * 解析 SEC 选项。
 *
 * @param options - 选项
 * @returns 冻结设置
 * @throws {ProviderDataError} User-Agent 缺失或 URL 非法
 */
export function resolveSecOptions(options: SecEdgarProviderOptions): ResolvedSecOptions {
  const id = options.id ?? 'sec-edgar';
  if (id.trim() === '') throw new ProviderDataError('sec-edgar', 'id 不能为空');
  if (
    typeof options.userAgent !== 'string' ||
    options.userAgent.trim() === '' ||
    !/(?:@|https?:\/\/)/u.test(options.userAgent)
  ) {
    throw new ProviderDataError(id, 'userAgent 必填，SEC 要求包含邮箱或 URL 联系方式');
  }
  const baseUrl = options.baseUrl ?? 'https://data.sec.gov';
  const tickersUrl = options.tickersUrl ?? 'https://www.sec.gov/files/company_tickers.json';
  if (!URL.canParse(baseUrl) || !URL.canParse(tickersUrl)) {
    throw new ProviderDataError(id, 'baseUrl/tickersUrl 非法');
  }
  return Object.freeze({
    id,
    baseUrl,
    tickersUrl,
    fetchImpl: options.fetchImpl,
    headers: Object.freeze({
      'user-agent': options.userAgent,
      accept: 'application/json, text/plain, */*',
      ...options.headers,
    }),
    timeoutMs: options.timeoutMs ?? 20_000,
    nowMs: options.nowMs ?? (() => Date.now()),
    currency: options.currency ?? createCurrency('USD', 2),
    venue: options.venue ?? 'sec',
    cikMap: Object.freeze({ ...options.cikMap }),
  });
}

/**
 * 拉取 SEC 公司 ticker 列表。
 *
 * @param settings - 设置
 * @param signal - 取消信号
 * @returns 按 ticker 排序的列表
 */
export async function fetchTickers(
  settings: ResolvedSecOptions,
  signal?: AbortSignal,
): Promise<readonly SecCompanyTicker[]> {
  const payload = await fetchJson(settings.tickersUrl, {
    providerId: settings.id,
    ...(settings.fetchImpl === undefined ? {} : { fetchImpl: settings.fetchImpl }),
    headers: settings.headers,
    timeoutMs: settings.timeoutMs,
    ...(signal === undefined ? {} : { signal }),
  });
  if (!isPlainObject(payload)) {
    throw new ProviderDataError(settings.id, 'ticker 列表必须是对象');
  }
  const tickers: SecCompanyTicker[] = [];
  for (const [key, value] of Object.entries(payload)) {
    if (!isPlainObject(value)) {
      throw new ProviderDataError(settings.id, `ticker 列表项 ${key} 必须是对象`);
    }
    const cikValue = value['cik_str'];
    const tickerValue = value['ticker'];
    const titleValue = value['title'];
    const cik = typeof cikValue === 'number' ? String(cikValue) : String(cikValue ?? '');
    if (cik === '' || typeof tickerValue !== 'string' || typeof titleValue !== 'string') {
      continue;
    }
    tickers.push(
      Object.freeze({
        cik: cik.padStart(10, '0'),
        ticker: tickerValue.toUpperCase(),
        title: titleValue,
      }),
    );
  }
  tickers.sort((left, right) => left.ticker.localeCompare(right.ticker));
  return Object.freeze(tickers);
}

/**
 * 把 ticker 映射为股票标的。
 *
 * @param settings - 设置
 * @param ticker - SEC ticker
 * @returns 标的
 */
export function instrumentForTicker(
  settings: ResolvedSecOptions,
  ticker: SecCompanyTicker,
): Instrument {
  return createInstrument({
    assetClass: AssetClass.Equity,
    venue: settings.venue,
    symbol: ticker.ticker,
    baseAsset: ticker.ticker,
    quoteCurrency: settings.currency,
    tickSize: Decimal.parse('0.01'),
    lotSize: Decimal.ONE,
  });
}

/**
 * 查找 instrument 对应的 SEC ticker。
 *
 * @param settings - 设置
 * @param tickers - ticker 列表
 * @param instrument - 标的
 * @returns ticker 或 undefined
 */
export function findTicker(
  tickers: readonly SecCompanyTicker[],
  instrument: Instrument,
): SecCompanyTicker | undefined {
  return tickers.find((ticker) => ticker.ticker === instrument.symbol.toUpperCase());
}

/**
 * 解析 CIK。
 *
 * @param settings - 设置
 * @param instrument - 标的
 * @param tickers - ticker 列表
 * @returns 10 位 CIK
 * @throws {ProviderDataError} 找不到 CIK
 */
export function resolveCik(
  settings: ResolvedSecOptions,
  instrument: Instrument,
  tickers: readonly SecCompanyTicker[],
): string {
  const key = instrumentKey(instrument);
  const explicit = settings.cikMap[key] ?? settings.cikMap[instrument.symbol.toUpperCase()];
  if (explicit !== undefined) {
    if (!/^\d{1,10}$/u.test(explicit)) {
      throw new ProviderDataError(settings.id, `cikMap 中的 CIK 非法: ${explicit}`);
    }
    return explicit.padStart(10, '0');
  }
  const ticker = findTicker(tickers, instrument);
  if (ticker === undefined) {
    throw new ProviderDataError(settings.id, `找不到 ${instrument.symbol} 的 CIK`);
  }
  return ticker.cik;
}
