/**
 * @file 美国财政部公开收益率曲线提供方。
 * @description 使用 home.treasury.gov 的 daily_treasury_yield_curve CSV；只读且仅支持 USD。
 * @module us-treasury/provider
 */

import {
  ProviderDataError,
  createCurrency,
  fetchText,
  isCurrency,
  type Currency,
  type FetchLike,
  type YieldCurve,
  type YieldCurvePoint,
  type YieldCurveProvider,
} from '@kurumi/finance';
import { Decimal } from '@kurumi/math';

/** 美国财政部提供方选项。 */
export interface UsTreasuryProviderOptions {
  /** 提供方 id，缺省 `us-treasury`。 */
  readonly id?: string;
  /** 基础地址，缺省 `https://home.treasury.gov`。 */
  readonly baseUrl?: string;
  /** 可注入 fetch。 */
  readonly fetchImpl?: FetchLike;
  /** 额外请求头。 */
  readonly headers?: Readonly<Record<string, string>>;
  /** 超时毫秒，缺省 20_000。 */
  readonly timeoutMs?: number;
  /** 当前时间注入。 */
  readonly nowMs?: () => number;
}

/** 解析后设置。 */
interface ResolvedUsTreasuryOptions {
  readonly id: string;
  readonly baseUrl: string;
  readonly fetchImpl: FetchLike | undefined;
  readonly headers: Readonly<Record<string, string>>;
  readonly timeoutMs: number;
  readonly nowMs: () => number;
}

/** 期限列到年数的映射。 */
const TENOR_YEARS: Readonly<Record<string, number>> = Object.freeze({
  '1 Mo': 1 / 12,
  '1.5 Month': 1.5 / 12,
  '2 Mo': 2 / 12,
  '3 Mo': 3 / 12,
  '4 Mo': 4 / 12,
  '6 Mo': 6 / 12,
  '1 Yr': 1,
  '2 Yr': 2,
  '3 Yr': 3,
  '5 Yr': 5,
  '7 Yr': 7,
  '10 Yr': 10,
  '20 Yr': 20,
  '30 Yr': 30,
});

/**
 * 创建美国财政部收益率曲线提供方。
 *
 * @param options - base URL、fetch、headers 与时间注入
 * @returns 只读 `YieldCurveProvider`
 *
 * @example
 * ```ts
 * const provider = createUsTreasuryProvider();
 * const curve = await provider.getLatest(createCurrency('USD', 2));
 * ```
 */
export function createUsTreasuryProvider(
  options: UsTreasuryProviderOptions = {},
): YieldCurveProvider {
  const settings = resolveOptions(options);
  return Object.freeze({
    id: settings.id,
    getLatest: (currency: Currency, signal?: AbortSignal) => getLatest(settings, currency, signal),
    getCurve: (date: string, currency: Currency, signal?: AbortSignal) =>
      getCurve(settings, date, currency, signal),
  });
}

/** 获取最新曲线。 */
async function getLatest(
  settings: ResolvedUsTreasuryOptions,
  currency: Currency,
  signal?: AbortSignal,
): Promise<YieldCurve> {
  assertUsd(settings.id, currency);
  const currentYear = new Date(settings.nowMs()).getUTCFullYear();
  for (const year of [currentYear, currentYear - 1, currentYear - 2]) {
    const rows = await fetchYear(settings, year, signal);
    if (rows.length > 0) return rows[0] as YieldCurve;
  }
  throw new ProviderDataError(settings.id, '最近三年没有可用收益率曲线');
}

/** 获取指定日期曲线。 */
async function getCurve(
  settings: ResolvedUsTreasuryOptions,
  date: string,
  currency: Currency,
  signal?: AbortSignal,
): Promise<YieldCurve> {
  assertUsd(settings.id, currency);
  const year = parseYear(date, settings.id);
  const rows = await fetchYear(settings, year, signal);
  const found = rows.find((row) => row.date === date);
  if (found === undefined) {
    throw new ProviderDataError(settings.id, `日期 ${date} 没有收益率曲线`);
  }
  return found;
}

/** 拉取并解析某年的 CSV。 */
async function fetchYear(
  settings: ResolvedUsTreasuryOptions,
  year: number,
  signal?: AbortSignal,
): Promise<readonly YieldCurve[]> {
  const url = new URL(
    `/resource-center/data-chart-center/interest-rates/daily-treasury-rates.csv/${year}/all`,
    settings.baseUrl,
  );
  url.searchParams.set('type', 'daily_treasury_yield_curve');
  url.searchParams.set('field_tdr_date_value', String(year));
  url.searchParams.set('_format', 'csv');
  const text = await fetchText(url.toString(), {
    providerId: settings.id,
    ...(settings.fetchImpl === undefined ? {} : { fetchImpl: settings.fetchImpl }),
    headers: {
      accept: 'text/csv, text/plain, */*',
      'user-agent': 'kurumi-finance-provider/0.1',
      ...settings.headers,
    },
    timeoutMs: settings.timeoutMs,
    ...(signal === undefined ? {} : { signal }),
  });
  return parseCsv(settings, text);
}

/** 解析 CSV。 */
function parseCsv(settings: ResolvedUsTreasuryOptions, text: string): readonly YieldCurve[] {
  const lines = text
    .split(/\r?\n/u)
    .map((line) => line.trim())
    .filter((line) => line !== '');
  const header = lines[0];
  if (header === undefined) return Object.freeze([]);
  const columns = header.split(',').map((item) => item.replace(/"/g, '').trim());
  const curves: YieldCurve[] = [];
  for (let index = 1; index < lines.length; index += 1) {
    const line = lines[index];
    if (line === undefined) continue;
    const cells = line.split(',').map((item) => item.replace(/"/g, '').trim());
    if (cells.length !== columns.length) {
      throw new ProviderDataError(settings.id, `CSV 第 ${index + 1} 行列数不一致`);
    }
    const date = cells[0];
    if (date === undefined) continue;
    const points: YieldCurvePoint[] = [];
    for (let column = 1; column < columns.length; column += 1) {
      const name = columns[column];
      const value = cells[column];
      if (name === undefined || value === undefined || value === '') continue;
      const tenor = TENOR_YEARS[name];
      if (tenor === undefined) continue;
      let percent: Decimal;
      try {
        percent = Decimal.parse(value);
      } catch (cause) {
        throw new ProviderDataError(
          settings.id,
          `CSV ${date} 的 ${name} 数值非法: ${value}`,
          cause,
        );
      }
      points.push(
        Object.freeze({
          tenorYears: tenor,
          yieldRate: percent.divide(Decimal.fromInteger(100), {
            scale: 8,
            rounding: 'half-even',
          }),
        }),
      );
    }
    if (points.length === 0) continue;
    points.sort((left, right) => left.tenorYears - right.tenorYears);
    curves.push(
      Object.freeze({
        date: formatUsDate(date, settings.id),
        currency: createCurrency('USD', 2),
        points: Object.freeze(points),
      }),
    );
  }
  // 明确最新在前，避免依赖数据源 CSV 行顺序；getLatest 取首行。
  curves.sort((left, right) => right.date.localeCompare(left.date));
  return Object.freeze(curves);
}

/** 解析选项。 */
function resolveOptions(options: UsTreasuryProviderOptions): ResolvedUsTreasuryOptions {
  const id = options.id ?? 'us-treasury';
  if (id.trim() === '') throw new ProviderDataError('us-treasury', 'id 不能为空');
  const baseUrl = options.baseUrl ?? 'https://home.treasury.gov';
  if (!URL.canParse(baseUrl)) throw new ProviderDataError(id, `baseUrl 非法: ${baseUrl}`);
  return Object.freeze({
    id,
    baseUrl,
    fetchImpl: options.fetchImpl,
    headers: Object.freeze({ ...options.headers }),
    timeoutMs: options.timeoutMs ?? 20_000,
    nowMs: options.nowMs ?? (() => Date.now()),
  });
}

/** 校验 USD。 */
function assertUsd(providerId: string, currency: Currency): void {
  if (!isCurrency(currency) || currency.code !== 'USD') {
    throw new ProviderDataError(providerId, '目前只支持 USD 收益率曲线');
  }
}

/** 解析日期年份。 */
function parseYear(date: string, providerId: string): number {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date);
  if (match === null) throw new ProviderDataError(providerId, `date 格式应为 YYYY-MM-DD`);
  const year = Number(match[1]);
  const month = Number(match[2]) - 1;
  const day = Number(match[3]);
  const parsed = new Date(Date.UTC(year, month, day));
  if (
    parsed.getUTCFullYear() !== year ||
    parsed.getUTCMonth() !== month ||
    parsed.getUTCDate() !== day
  ) {
    throw new ProviderDataError(providerId, `日期非法: ${date}`);
  }
  return year;
}

/** 把 MM/DD/YYYY 转为 YYYY-MM-DD。 */
function formatUsDate(value: string, providerId: string): string {
  const match = /^(\d{1,2})\/(\d{1,2})\/(\d{4})$/.exec(value);
  if (match === null) throw new ProviderDataError(providerId, `CSV 日期非法: ${value}`);
  const year = Number(match[3]);
  const month = Number(match[1]) - 1;
  const day = Number(match[2]);
  const parsed = new Date(Date.UTC(year, month, day));
  if (
    parsed.getUTCFullYear() !== year ||
    parsed.getUTCMonth() !== month ||
    parsed.getUTCDate() !== day
  ) {
    throw new ProviderDataError(providerId, `CSV 日期非法: ${value}`);
  }
  return parsed.toISOString().slice(0, 10);
}
