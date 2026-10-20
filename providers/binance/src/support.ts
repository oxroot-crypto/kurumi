/**
 * @file Binance 提供方共享选项、货币映射与解析工具。
 * @description 统一现货/期货 base URL、fetch 注入、交易对映射与 K 线转换。
 * @module binance/support
 */

import {
  BarInterval,
  ProviderDataError,
  barIntervalMs,
  createBar,
  createCurrency,
  fetchJson,
  type Bar,
  type Currency,
  type FetchLike,
  type Instrument,
} from '@kurumi/finance';
import { Decimal } from '@kurumi/math';

/** 默认货币精度。 */
const DEFAULT_CURRENCIES: Readonly<Record<string, number>> = Object.freeze({
  USDT: 8,
  USDC: 8,
  BUSD: 8,
  FDUSD: 8,
  TUSD: 8,
  BTC: 8,
  ETH: 8,
  BNB: 8,
  USD: 2,
  EUR: 2,
  GBP: 2,
  JPY: 0,
});

/** Binance 提供方选项。 */
export interface BinanceProviderOptions {
  /** 提供方 id，缺省 `binance`。 */
  readonly id?: string;
  /** 现货 API 根地址，缺省 `https://api.binance.com`。 */
  readonly baseUrl?: string;
  /** 期货 API 根地址，缺省 `https://fapi.binance.com`。 */
  readonly futuresBaseUrl?: string;
  /** 可注入 fetch。 */
  readonly fetchImpl?: FetchLike;
  /** 额外请求头。 */
  readonly headers?: Readonly<Record<string, string>>;
  /** 超时毫秒，缺省 15_000。 */
  readonly timeoutMs?: number;
  /** 当前时间注入。 */
  readonly nowMs?: () => number;
  /** 货币精度覆盖。 */
  readonly currencies?: Readonly<Record<string, number>>;
  /** 现货 venue，缺省 `binance`。 */
  readonly venue?: string;
  /** 期货 venue，缺省 `binance-futures`。 */
  readonly futuresVenue?: string;
}

/** 解析后的 Binance 设置。 */
export interface ResolvedBinanceOptions {
  readonly id: string;
  readonly baseUrl: string;
  readonly futuresBaseUrl: string;
  readonly fetchImpl: FetchLike | undefined;
  readonly headers: Readonly<Record<string, string>>;
  readonly timeoutMs: number;
  readonly nowMs: () => number;
  readonly currencies: Readonly<Record<string, number>>;
  readonly venue: string;
  readonly futuresVenue: string;
}

/**
 * 解析 Binance 选项。
 *
 * @param options - 原始选项
 * @returns 冻结设置
 * @throws {ProviderDataError} 选项非法
 */
export function resolveBinanceOptions(
  options: BinanceProviderOptions = {},
): ResolvedBinanceOptions {
  const id = options.id ?? 'binance';
  if (id.trim() === '') throw new ProviderDataError('binance', 'id 不能为空');
  const baseUrl = options.baseUrl ?? 'https://api.binance.com';
  const futuresBaseUrl = options.futuresBaseUrl ?? 'https://fapi.binance.com';
  if (!URL.canParse(baseUrl) || !URL.canParse(futuresBaseUrl)) {
    throw new ProviderDataError(id, 'baseUrl/futuresBaseUrl 非法');
  }
  return Object.freeze({
    id,
    baseUrl,
    futuresBaseUrl,
    fetchImpl: options.fetchImpl,
    headers: Object.freeze({ 'user-agent': 'kurumi-finance-provider/0.1', ...options.headers }),
    timeoutMs: options.timeoutMs ?? 15_000,
    nowMs: options.nowMs ?? (() => Date.now()),
    currencies: Object.freeze({ ...DEFAULT_CURRENCIES, ...options.currencies }),
    venue: options.venue ?? 'binance',
    futuresVenue: options.futuresVenue ?? 'binance-futures',
  });
}

/**
 * 请求 Binance JSON。
 *
 * @param settings - 设置
 * @param baseUrl - 现货或期货根地址
 * @param path - API 路径
 * @param query - 查询参数
 * @param signal - 取消信号
 * @returns JSON
 */
export async function getBinanceJson(
  settings: ResolvedBinanceOptions,
  baseUrl: string,
  path: string,
  query: Readonly<Record<string, string | number>>,
  signal?: AbortSignal,
): Promise<unknown> {
  const url = new URL(path, baseUrl);
  for (const [key, value] of Object.entries(query)) {
    url.searchParams.set(key, String(value));
  }
  return fetchJson(url.toString(), {
    providerId: settings.id,
    ...(settings.fetchImpl === undefined ? {} : { fetchImpl: settings.fetchImpl }),
    headers: settings.headers,
    timeoutMs: settings.timeoutMs,
    ...(signal === undefined ? {} : { signal }),
  });
}

/**
 * 把标的映射为 Binance symbol。
 *
 * @param instrument - 标的
 * @returns 去掉分隔符后的 symbol
 */
export function toBinanceSymbol(instrument: Instrument): string {
  const raw = instrument.symbol.replace(/[/-]/g, '').toUpperCase();
  return raw.endsWith('PERP') ? raw.slice(0, -4) : raw;
}

/**
 * 解析货币。
 *
 * @param settings - 设置
 * @param code - 货币代码
 * @returns Currency
 * @throws {ProviderDataError} 代码非法
 */
export function currencyFor(settings: ResolvedBinanceOptions, code: string): Currency {
  try {
    return createCurrency(code, settings.currencies[code] ?? 8);
  } catch (cause) {
    throw new ProviderDataError(settings.id, `无法识别货币 ${code}`, cause);
  }
}

/**
 * 解析必需 Decimal。
 *
 * @param value - 原始值
 * @param path - 字段路径
 * @param providerId - 提供方 id
 * @returns Decimal
 */
export function requireDecimal(value: unknown, path: string, providerId: string): Decimal {
  const parsed = optionalDecimal(value, path, providerId);
  if (parsed === undefined) {
    throw new ProviderDataError(providerId, `${path} 缺失或非法`);
  }
  return parsed;
}

/**
 * 解析可选 Decimal。
 *
 * @param value - 原始值
 * @param path - 字段路径
 * @param providerId - 提供方 id
 * @returns Decimal 或 undefined
 */
export function optionalDecimal(
  value: unknown,
  path: string,
  providerId: string,
): Decimal | undefined {
  if (value === null || value === undefined || value === '') return undefined;
  if (typeof value !== 'string' && typeof value !== 'number') {
    throw new ProviderDataError(providerId, `${path} 必须是字符串或数字`);
  }
  try {
    return Decimal.parse(String(value));
  } catch (cause) {
    throw new ProviderDataError(providerId, `${path} 不是合法数值: ${String(value)}`, cause);
  }
}

/**
 * 解析必需正整数。
 *
 * @param value - 原始值
 * @param path - 字段路径
 * @param providerId - 提供方 id
 * @returns 正整数
 */
export function requireInteger(value: unknown, path: string, providerId: string): number {
  const numberValue = typeof value === 'string' ? Number(value) : value;
  if (typeof numberValue !== 'number' || !Number.isSafeInteger(numberValue) || numberValue < 0) {
    throw new ProviderDataError(providerId, `${path} 必须是非负安全整数`);
  }
  return numberValue;
}

/**
 * 从 K 线数组构建 Bar。
 *
 * @param settings - 设置
 * @param instrument - 标的
 * @param interval - 周期
 * @param kline - Binance K 线数组
 * @returns Bar
 */
export function buildBarFromKline(
  settings: ResolvedBinanceOptions,
  instrument: Instrument,
  interval: BarInterval,
  kline: readonly unknown[],
): Bar {
  if (kline.length < 9) {
    throw new ProviderDataError(settings.id, 'K 线数组长度不足 9');
  }
  const openTimeMs = requireInteger(kline[0], 'kline[0]', settings.id);
  const open = requireDecimal(kline[1], 'kline[1]', settings.id);
  const high = requireDecimal(kline[2], 'kline[2]', settings.id);
  const low = requireDecimal(kline[3], 'kline[3]', settings.id);
  const close = requireDecimal(kline[4], 'kline[4]', settings.id);
  const volume = requireDecimal(kline[5], 'kline[5]', settings.id);
  const tradeCount = requireInteger(kline[8], 'kline[8]', settings.id);
  return createBar({
    instrument,
    interval,
    openTimeMs,
    closeTimeMs: openTimeMs + barIntervalMs(interval),
    open,
    high,
    low,
    close,
    volume,
    tradeCount,
  });
}

/**
 * 校验数组。
 *
 * @param value - 原始值
 * @param path - 字段路径
 * @param providerId - 提供方 id
 * @returns 数组
 */
export function requireArray(value: unknown, path: string, providerId: string): readonly unknown[] {
  if (!Array.isArray(value)) {
    throw new ProviderDataError(providerId, `${path} 必须是数组`);
  }
  return value;
}

/**
 * 校验对象。
 *
 * @param value - 原始值
 * @param path - 字段路径
 * @param providerId - 提供方 id
 * @returns 普通对象
 */
export function requireRecord(
  value: unknown,
  path: string,
  providerId: string,
): Record<string, unknown> {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new ProviderDataError(providerId, `${path} 必须是对象`);
  }
  return value as Record<string, unknown>;
}
