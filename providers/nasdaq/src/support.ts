/**
 * @file Nasdaq 提供方共享选项与解析工具。
 * @description 统一 base URL、fetch 注入、字段解析、日期与错误包装。
 * @module nasdaq/support
 */

import { isPlainObject } from '@kurumi/core';
import { ProviderDataError, fetchJson, type FetchLike } from '@kurumi/finance';
import { Decimal } from '@kurumi/math';

/** Nasdaq 资产类别参数。 */
export type NasdaqAssetClass = 'stocks' | 'etf' | 'index';

/** Nasdaq 提供方选项。 */
export interface NasdaqProviderOptions {
  /** 提供方 id，缺省 `nasdaq`。 */
  readonly id?: string;
  /** API 根地址，缺省 `https://api.nasdaq.com`。 */
  readonly baseUrl?: string;
  /** 可注入 fetch。 */
  readonly fetchImpl?: FetchLike;
  /** 额外请求头。 */
  readonly headers?: Readonly<Record<string, string>>;
  /** 超时毫秒，缺省 15_000。 */
  readonly timeoutMs?: number;
  /** 当前时间注入，便于测试。 */
  readonly nowMs?: () => number;
  /** 股票/ETF/指数资产类别，缺省 stocks。 */
  readonly assetClass?: NasdaqAssetClass;
  /** 单次请求最大行数，缺省 50_000。 */
  readonly maxRows?: number;
}

/** 解析后的提供方设置。 */
export interface ResolvedNasdaqOptions {
  readonly id: string;
  readonly baseUrl: string;
  readonly fetchImpl: FetchLike | undefined;
  readonly headers: Readonly<Record<string, string>>;
  readonly timeoutMs: number;
  readonly nowMs: () => number;
  readonly assetClass: NasdaqAssetClass;
  readonly maxRows: number;
}

/**
 * 解析 Nasdaq 选项。
 *
 * @param options - 原始选项
 * @returns 冻结设置
 * @throws {ProviderDataError} id/baseUrl/tickSize/maxRows 非法
 */
export function resolveNasdaqOptions(options: NasdaqProviderOptions = {}): ResolvedNasdaqOptions {
  const id = options.id ?? 'nasdaq';
  if (id.trim() === '') throw new ProviderDataError('nasdaq', 'id 不能为空');
  const baseUrl = options.baseUrl ?? 'https://api.nasdaq.com';
  if (!URL.canParse(baseUrl)) {
    throw new ProviderDataError(id, `baseUrl 非法: ${baseUrl}`);
  }
  const maxRows = options.maxRows ?? 50_000;
  if (!Number.isSafeInteger(maxRows) || maxRows <= 0) {
    throw new ProviderDataError(id, 'maxRows 必须是正安全整数');
  }
  const assetClass = options.assetClass ?? 'stocks';
  if (assetClass !== 'stocks' && assetClass !== 'etf' && assetClass !== 'index') {
    throw new ProviderDataError(id, `assetClass 非法: ${String(assetClass)}`);
  }
  return Object.freeze({
    id,
    baseUrl,
    fetchImpl: options.fetchImpl,
    headers: Object.freeze({ 'user-agent': 'kurumi-finance-provider/0.1', ...options.headers }),
    timeoutMs: options.timeoutMs ?? 15_000,
    nowMs: options.nowMs ?? (() => Date.now()),
    assetClass,
    maxRows,
  });
}

/**
 * 请求 Nasdaq JSON。
 *
 * @param settings - 解析后的设置
 * @param path - API 路径
 * @param query - 查询参数
 * @param signal - 取消信号
 * @returns JSON 响应
 */
export async function getNasdaqJson(
  settings: ResolvedNasdaqOptions,
  path: string,
  query: Readonly<Record<string, string>>,
  signal?: AbortSignal,
): Promise<unknown> {
  const url = new URL(path, settings.baseUrl);
  for (const [key, value] of Object.entries(query)) {
    url.searchParams.set(key, value);
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
 * 读取普通对象字段。
 *
 * @param value - 待检查值
 * @param path - 字段路径
 * @param providerId - 提供方 id
 * @returns 普通对象
 * @throws {ProviderDataError} 不是普通对象
 */
export function requireRecord(
  value: unknown,
  path: string,
  providerId: string,
): Record<string, unknown> {
  if (!isPlainObject(value)) {
    throw new ProviderDataError(providerId, `${path} 必须是对象`);
  }
  return value;
}

/**
 * 读取数组字段。
 *
 * @param value - 待检查值
 * @param path - 字段路径
 * @param providerId - 提供方 id
 * @returns 数组
 * @throws {ProviderDataError} 不是数组
 */
export function requireArray(value: unknown, path: string, providerId: string): readonly unknown[] {
  if (!Array.isArray(value)) {
    throw new ProviderDataError(providerId, `${path} 必须是数组`);
  }
  return value;
}

/**
 * 解析 Nasdaq 金额/数量字符串。
 *
 * @param value - 原始值
 * @param path - 字段路径
 * @param providerId - 提供方 id
 * @returns Decimal；`--` / `N/A` / 空串返回 undefined
 * @throws {ProviderDataError} 非法数字
 */
export function parseDecimalField(
  value: unknown,
  path: string,
  providerId: string,
): Decimal | undefined {
  if (value === null || value === undefined) return undefined;
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) {
      throw new ProviderDataError(providerId, `${path} 不是有限数`);
    }
    return Decimal.parse(String(value));
  }
  if (typeof value !== 'string') {
    throw new ProviderDataError(providerId, `${path} 必须是字符串或数字`);
  }
  const normalized = value.trim().replace(/[$,%]/g, '');
  if (normalized === '' || normalized === '--' || normalized.toUpperCase() === 'N/A') {
    return undefined;
  }
  try {
    return Decimal.parse(normalized);
  } catch (cause) {
    throw new ProviderDataError(providerId, `${path} 不是合法数值: ${value}`, cause);
  }
}

/**
 * 解析必需金额字段。
 *
 * @param value - 原始值
 * @param path - 字段路径
 * @param providerId - 提供方 id
 * @returns Decimal
 * @throws {ProviderDataError} 缺失或非法
 */
export function requireDecimalField(value: unknown, path: string, providerId: string): Decimal {
  const parsed = parseDecimalField(value, path, providerId);
  if (parsed === undefined) {
    throw new ProviderDataError(providerId, `${path} 缺失`);
  }
  return parsed;
}

/**
 * 解析 `MM/DD/YYYY` 为 UTC 毫秒。
 *
 * @param value - 日期字符串
 * @param path - 字段路径
 * @param providerId - 提供方 id
 * @returns UTC 毫秒
 */
export function parseSlashDate(value: unknown, path: string, providerId: string): number {
  if (typeof value !== 'string') {
    throw new ProviderDataError(providerId, `${path} 必须是日期字符串`);
  }
  const match = /^(\d{1,2})\/(\d{1,2})\/(\d{4})$/.exec(value.trim());
  if (match === null) {
    throw new ProviderDataError(providerId, `${path} 日期格式应为 MM/DD/YYYY: ${value}`);
  }
  const month = Number(match[1]) - 1;
  const day = Number(match[2]);
  const year = Number(match[3]);
  const timestamp = Date.UTC(year, month, day);
  const parsed = new Date(timestamp);
  if (
    parsed.getUTCFullYear() !== year ||
    parsed.getUTCMonth() !== month ||
    parsed.getUTCDate() !== day
  ) {
    throw new ProviderDataError(providerId, `${path} 日期非法: ${value}`);
  }
  return timestamp;
}

/**
 * 解析 `October 20, 2026` 为 UTC 毫秒。
 *
 * @param value - 日期字符串
 * @param providerId - 提供方 id
 * @returns UTC 毫秒
 */
export function parseLongDate(value: string, providerId: string): number {
  const match = /^([A-Za-z]+)\s+(\d{1,2}),\s*(\d{4})$/.exec(value.trim());
  if (match === null) {
    throw new ProviderDataError(providerId, `无法解析到期日: ${value}`);
  }
  const month = MONTHS[match[1]?.toLowerCase() ?? ''];
  const day = Number(match[2]);
  const year = Number(match[3]);
  if (month === undefined) {
    throw new ProviderDataError(providerId, `未知月份: ${String(match[1])}`);
  }
  const timestamp = Date.UTC(year, month, day);
  const parsed = new Date(timestamp);
  if (
    parsed.getUTCFullYear() !== year ||
    parsed.getUTCMonth() !== month ||
    parsed.getUTCDate() !== day
  ) {
    throw new ProviderDataError(providerId, `日期非法: ${value}`);
  }
  return timestamp;
}

/**
 * 把 epoch 毫秒格式化为 `YYYY-MM-DD`。
 *
 * @param timestampMs - 时间
 * @returns UTC 日期
 */
export function formatDate(timestampMs: number): string {
  return new Date(timestampMs).toISOString().slice(0, 10);
}

/**
 * 解析 Nasdaq 文本日期（可选带时间）为时间戳；失败时回退当前时间。
 *
 * @param value - 文本
 * @param fallbackMs - 回退时间
 * @param providerId - 提供方 id
 * @returns 时间戳
 */
export function parseFlexibleTimestamp(
  value: unknown,
  fallbackMs: number,
  providerId: string,
): number {
  if (typeof value !== 'string' || value.trim() === '') return fallbackMs;
  const normalized = value.replace('AS OF ', '').replace(/\s+/g, ' ').trim();
  // Nasdaq 可能返回 "Oct 20, 2026 4:53 AM ET"；日期用于时间戳，盘口时间本身不参与行情计算。
  const full = /^([A-Za-z]+)\s+(\d{1,2}),\s*(\d{4})/.exec(normalized);
  if (full !== null) {
    const month = MONTHS[full[1]?.toLowerCase() ?? ''];
    if (month !== undefined) return Date.UTC(Number(full[3]), month, Number(full[2]));
  }
  const short = /^([A-Za-z]+)\s+(\d{1,2})$/u.exec(normalized);
  if (short !== null) {
    const month = MONTHS[short[1]?.toLowerCase() ?? ''];
    if (month !== undefined)
      return Date.UTC(new Date(fallbackMs).getUTCFullYear(), month, Number(short[2]));
  }
  throw new ProviderDataError(providerId, `无法解析时间: ${value}`);
}

/** 月份映射。 */
const MONTHS: Readonly<Record<string, number>> = Object.freeze({
  january: 0,
  february: 1,
  march: 2,
  april: 3,
  may: 4,
  june: 5,
  july: 6,
  august: 7,
  september: 8,
  october: 9,
  november: 10,
  december: 11,
  jan: 0,
  feb: 1,
  mar: 2,
  apr: 3,
  jun: 5,
  jul: 6,
  aug: 7,
  sep: 8,
  sept: 8,
  oct: 9,
  nov: 10,
  dec: 11,
});
