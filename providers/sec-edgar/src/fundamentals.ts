/**
 * @file SEC EDGAR 只读基本面提供方。
 * @description 从 companyfacts XBRL 中提取最近年度/季度报表，使用 residuals 补齐明细行并校验资产恒等式。
 * @module sec-edgar/fundamentals
 */

import { isPlainObject } from '@kurumi/core';
import {
  ProviderDataError,
  StatementPeriodType,
  createCompanyProfile,
  createFinancialStatements,
  fetchJson,
  type CompanyProfile,
  type FinancialStatements,
  type FundamentalDataProvider,
  type GetFinancialStatementsOptions,
  type Instrument,
} from '@kurumi/finance';
import { Decimal } from '@kurumi/math';

import {
  fetchTickers,
  findTicker,
  resolveCik,
  resolveSecOptions,
  type ResolvedSecOptions,
  type SecEdgarProviderOptions,
} from './support.js';

/** XBRL fact。 */
interface SecFact {
  readonly start?: string;
  readonly end: string;
  readonly val: number | string;
  readonly fy?: number;
  readonly fp?: string;
  readonly form?: string;
  readonly filed?: string;
}

/** fact 索引。 */
interface FactIndex {
  readonly facts: Readonly<Record<string, unknown>>;
}

/** 期间。 */
interface StatementPeriod {
  readonly end: string;
  readonly start: string;
  readonly fy: number;
  readonly form: '10-K' | '10-Q';
}

/** 年度/季度目标天数。 */
const ANNUAL_DAYS = 365;
const QUARTER_DAYS = 90;

/**
 * 创建 SEC EDGAR 基本面提供方。
 *
 * @param options - User-Agent、CIK 映射与数据地址
 * @returns 只读 `FundamentalDataProvider`
 */
export function createSecEdgarFundamentalProvider(
  options: SecEdgarProviderOptions,
): FundamentalDataProvider {
  const settings = resolveSecOptions(options);
  return Object.freeze({
    id: settings.id,
    getCompanyProfile: async (instrument: Instrument, signal?: AbortSignal) =>
      getCompanyProfile(settings, instrument, signal),
    getFinancialStatements: async (instrument: Instrument, query?: GetFinancialStatementsOptions) =>
      getFinancialStatements(settings, instrument, query),
  });
}

/** 获取公司资料。 */
async function getCompanyProfile(
  settings: ResolvedSecOptions,
  instrument: Instrument,
  signal?: AbortSignal,
): Promise<CompanyProfile | undefined> {
  const tickers = await fetchTickers(settings, signal);
  const ticker = findTicker(tickers, instrument);
  if (ticker === undefined) return undefined;
  return createCompanyProfile({
    instrument,
    name: ticker.title,
    country: 'US',
    currency: settings.currency,
  });
}

/** 获取财务报表。 */
async function getFinancialStatements(
  settings: ResolvedSecOptions,
  instrument: Instrument,
  options?: GetFinancialStatementsOptions,
): Promise<readonly FinancialStatements[]> {
  const periodType = options?.periodType ?? StatementPeriodType.Annual;
  if (periodType === StatementPeriodType.Ttm) {
    throw new ProviderDataError(settings.id, 'companyfacts 不直接提供 TTM，请由季度数据合成');
  }
  const periodsCount = options?.periods ?? 5;
  if (!Number.isSafeInteger(periodsCount) || periodsCount <= 0) {
    throw new ProviderDataError(settings.id, 'periods 必须是正安全整数');
  }
  const tickers = await fetchTickers(settings, options?.signal);
  const cik = resolveCik(settings, instrument, tickers);
  const index = await fetchCompanyFacts(settings, cik, options?.signal);
  const form = periodType === StatementPeriodType.Annual ? '10-K' : '10-Q';
  const periods = collectPeriods(index, form, periodsCount, settings.id);
  const statements: FinancialStatements[] = [];
  for (const period of periods) {
    try {
      statements.push(buildStatements(settings, index, period, periodType));
    } catch (cause) {
      if (cause instanceof ProviderDataError) throw cause;
      throw new ProviderDataError(
        settings.id,
        `无法构建 ${instrument.symbol} ${period.end} 报表`,
        cause,
      );
    }
  }
  return Object.freeze(statements);
}

/** 拉取 companyfacts。 */
async function fetchCompanyFacts(
  settings: ResolvedSecOptions,
  cik: string,
  signal?: AbortSignal,
): Promise<FactIndex> {
  const url = new URL(`/api/xbrl/companyfacts/CIK${cik}.json`, settings.baseUrl);
  const payload = await fetchJson(url.toString(), {
    providerId: settings.id,
    ...(settings.fetchImpl === undefined ? {} : { fetchImpl: settings.fetchImpl }),
    headers: settings.headers,
    timeoutMs: settings.timeoutMs,
    ...(signal === undefined ? {} : { signal }),
  });
  if (!isPlainObject(payload)) {
    throw new ProviderDataError(settings.id, 'companyfacts 必须是对象');
  }
  const facts = payload['facts'];
  if (!isPlainObject(facts)) {
    throw new ProviderDataError(settings.id, 'companyfacts.facts 必须是对象');
  }
  return { facts };
}

/** 收集报告期。 */
function collectPeriods(
  index: FactIndex,
  form: '10-K' | '10-Q',
  count: number,
  providerId: string,
): readonly StatementPeriod[] {
  const targetDays = form === '10-K' ? ANNUAL_DAYS : QUARTER_DAYS;
  const facts = [
    ...entriesFor(index, 'us-gaap', 'NetIncomeLoss', 'USD'),
    ...entriesFor(index, 'us-gaap', 'ProfitLoss', 'USD'),
  ];
  const byEnd = new Map<string, SecFact>();
  for (const fact of facts) {
    if (fact.form === undefined || !fact.form.startsWith(form) || fact.end === undefined) continue;
    const existing = byEnd.get(fact.end);
    if (existing === undefined || isCloserPeriod(fact, existing, targetDays)) {
      byEnd.set(fact.end, fact);
    }
  }
  const result: StatementPeriod[] = [];
  for (const [end, fact] of byEnd) {
    const start =
      fact.start ?? shiftDate(end, form === '10-K' ? -(ANNUAL_DAYS - 1) : -(QUARTER_DAYS - 1));
    const fy = typeof fact.fy === 'number' ? fact.fy : Number(end.slice(0, 4));
    result.push({ end, start, fy, form });
  }
  result.sort((left, right) => right.end.localeCompare(left.end));
  if (result.length === 0) {
    throw new ProviderDataError(providerId, '没有可用的 10-K/10-Q 净利润数据');
  }
  return Object.freeze(result.slice(0, count));
}

/**
 * 判断候选 fact 是否更接近目标期间长度。
 *
 * @remarks
 * 10-Q 的 companyfacts 同时包含单季度与年初至今 fact；只按 end 去重会把两者混为一谈。
 */
function isCloserPeriod(candidate: SecFact, existing: SecFact, targetDays: number): boolean {
  const candidateDistance = periodDistanceDays(candidate, targetDays);
  const existingDistance = periodDistanceDays(existing, targetDays);
  if (candidateDistance < existingDistance) return true;
  if (candidateDistance > existingDistance) return false;
  return (candidate.filed ?? '') > (existing.filed ?? '');
}

/** 计算 fact 期间与目标天数的距离；缺少 start 的 instant fact 优先级最低。 */
function periodDistanceDays(fact: SecFact, targetDays: number): number {
  if (fact.start === undefined) return Number.POSITIVE_INFINITY;
  return Math.abs(daysBetweenIso(fact.start, fact.end) - targetDays);
}

/** 构建单个期间报表。 */
function buildStatements(
  settings: ResolvedSecOptions,
  index: FactIndex,
  period: StatementPeriod,
  periodType: StatementPeriodType,
): FinancialStatements {
  const targetDays = period.form === '10-K' ? ANNUAL_DAYS : QUARTER_DAYS;
  const revenue = requireMetric(
    settings,
    index,
    [
      'RevenueFromContractWithCustomerExcludingAssessedTax',
      'Revenues',
      'SalesRevenueNet',
      'RevenueFromContractWithCustomerIncludingAssessedTax',
    ],
    'USD',
    period.end,
    period.form,
    targetDays,
    'revenue',
  );
  const costOfRevenue =
    metric(
      settings,
      index,
      ['CostOfRevenue', 'CostOfGoodsAndServicesSold', 'CostOfGoodsSold'],
      'USD',
      period.end,
      period.form,
      targetDays,
    ) ?? Decimal.ZERO;
  const operatingIncomeFact = metric(
    settings,
    index,
    ['OperatingIncomeLoss'],
    'USD',
    period.end,
    period.form,
    targetDays,
  );
  const operatingExpensesFact = metric(
    settings,
    index,
    ['OperatingExpenses'],
    'USD',
    period.end,
    period.form,
    targetDays,
  );
  const operatingIncome =
    operatingIncomeFact ??
    revenue.subtract(costOfRevenue).subtract(operatingExpensesFact ?? Decimal.ZERO);
  const operatingExpenses =
    operatingExpensesFact ?? maxZero(revenue.subtract(costOfRevenue).subtract(operatingIncome));
  const interestExpense = maxZero(
    metric(
      settings,
      index,
      ['InterestExpense', 'InterestExpenseNonoperating', 'InterestAndDebtExpense'],
      'USD',
      period.end,
      period.form,
      targetDays,
    ) ?? Decimal.ZERO,
  );
  const taxExpense = maxZero(
    metric(
      settings,
      index,
      ['IncomeTaxExpenseBenefit'],
      'USD',
      period.end,
      period.form,
      targetDays,
    ) ?? Decimal.ZERO,
  );
  const netIncome = requireMetric(
    settings,
    index,
    ['NetIncomeLoss', 'ProfitLoss'],
    'USD',
    period.end,
    period.form,
    targetDays,
    'netIncome',
  );
  const pretaxIncome = netIncome.add(taxExpense);
  const otherIncome = pretaxIncome.add(interestExpense).subtract(operatingIncome);
  const shares = requireShares(settings, index, period.end, period.form, targetDays);
  const epsBasic = metric(
    settings,
    index,
    ['EarningsPerShareBasic'],
    'USD/shares',
    period.end,
    period.form,
    targetDays,
  );
  const epsDiluted = metric(
    settings,
    index,
    ['EarningsPerShareDiluted'],
    'USD/shares',
    period.end,
    period.form,
    targetDays,
  );

  const cashFlow = {
    operatingCashFlow:
      metric(
        settings,
        index,
        [
          'NetCashProvidedByUsedInOperatingActivities',
          'NetCashProvidedByUsedInOperatingActivitiesContinuingOperations',
        ],
        'USD',
        period.end,
        period.form,
        targetDays,
      ) ?? Decimal.ZERO,
    capitalExpenditure: maxZero(
      metric(
        settings,
        index,
        ['PaymentsToAcquirePropertyPlantAndEquipment', 'PaymentsToAcquireProductiveAssets'],
        'USD',
        period.end,
        period.form,
        targetDays,
      ) ?? Decimal.ZERO,
    ),
    investingCashFlow:
      metric(
        settings,
        index,
        ['NetCashProvidedByUsedInInvestingActivities'],
        'USD',
        period.end,
        period.form,
        targetDays,
      ) ?? Decimal.ZERO,
    financingCashFlow:
      metric(
        settings,
        index,
        ['NetCashProvidedByUsedInFinancingActivities'],
        'USD',
        period.end,
        period.form,
        targetDays,
      ) ?? Decimal.ZERO,
    depreciationAmortization: maxZero(
      metric(
        settings,
        index,
        [
          'DepreciationDepletionAndAmortization',
          'DepreciationAmortizationAndAccretionNet',
          'Depreciation',
        ],
        'USD',
        period.end,
        period.form,
        targetDays,
      ) ?? Decimal.ZERO,
    ),
    dividendsPaid: maxZero(
      metric(
        settings,
        index,
        ['PaymentsOfDividends', 'PaymentsOfDividendsCommonStock'],
        'USD',
        period.end,
        period.form,
        targetDays,
      ) ?? Decimal.ZERO,
    ),
    shareRepurchases: maxZero(
      metric(
        settings,
        index,
        ['PaymentsForRepurchaseOfCommonStock'],
        'USD',
        period.end,
        period.form,
        targetDays,
      ) ?? Decimal.ZERO,
    ),
  };

  const balance = buildBalance(settings, index, period, targetDays);

  return createFinancialStatements({
    period: {
      fiscalYear: period.fy,
      periodType,
      startMs: parseDay(period.start, settings.id),
      endMs: parseDay(period.end, settings.id),
      currency: settings.currency,
    },
    income: {
      revenue,
      costOfRevenue,
      operatingExpenses,
      interestExpense,
      otherIncome,
      taxExpense,
      sharesOutstanding: shares,
      ...(epsBasic === undefined ? {} : { epsBasic }),
      ...(epsDiluted === undefined ? {} : { epsDiluted }),
    },
    balance,
    cashFlow,
  });
}

/** 构建资产负债表，使用 residuals 补齐明细。 */
function buildBalance(
  settings: ResolvedSecOptions,
  index: FactIndex,
  period: StatementPeriod,
  targetDays: number,
): {
  readonly cashAndEquivalents: Decimal;
  readonly shortTermInvestments: Decimal;
  readonly accountsReceivable: Decimal;
  readonly inventory: Decimal;
  readonly otherCurrentAssets: Decimal;
  readonly propertyPlantEquipment: Decimal;
  readonly goodwill: Decimal;
  readonly intangibleAssets: Decimal;
  readonly otherNonCurrentAssets: Decimal;
  readonly accountsPayable: Decimal;
  readonly shortTermDebt: Decimal;
  readonly otherCurrentLiabilities: Decimal;
  readonly longTermDebt: Decimal;
  readonly otherNonCurrentLiabilities: Decimal;
  readonly shareholdersEquity: Decimal;
  readonly retainedEarnings: Decimal;
} {
  const metricValue = (tags: readonly string[], label: string): Decimal => {
    const value = metric(settings, index, tags, 'USD', period.end, period.form, targetDays);
    if (value === undefined) {
      throw new ProviderDataError(settings.id, `缺少资产负债表字段 ${label}（${period.end}）`);
    }
    return value;
  };
  const optional = (tags: readonly string[]): Decimal =>
    metric(settings, index, tags, 'USD', period.end, period.form, targetDays) ?? Decimal.ZERO;

  const currentAssets = metricValue(['AssetsCurrent'], 'AssetsCurrent');
  const totalAssets = metricValue(['Assets'], 'Assets');
  const currentLiabilities = metricValue(['LiabilitiesCurrent'], 'LiabilitiesCurrent');
  const totalLiabilities = metricValue(['Liabilities'], 'Liabilities');
  const equity = metricValue(
    [
      'StockholdersEquity',
      'StockholdersEquityIncludingPortionAttributableToNoncontrollingInterest',
    ],
    'StockholdersEquity',
  );
  const cash = optional(['CashAndCashEquivalentsAtCarryingValue']);
  const shortTermInvestments = optional(['ShortTermInvestments', 'MarketableSecuritiesCurrent']);
  const receivables = optional(['AccountsReceivableNetCurrent', 'ReceivablesNetCurrent']);
  const inventory = optional(['InventoryNet']);
  const ppe = optional(['PropertyPlantAndEquipmentNet']);
  const goodwill = optional(['Goodwill']);
  const intangibles = optional([
    'IntangibleAssetsNetExcludingGoodwill',
    'FiniteLivedIntangibleAssetsNet',
  ]);
  const accountsPayable = optional(['AccountsPayableCurrent']);
  const shortTermDebt = optional(['ShortTermBorrowings', 'LongTermDebtCurrent', 'DebtCurrent']);
  const longTermDebt = optional(['LongTermDebtNoncurrent', 'LongTermDebt']);
  const retainedEarnings = optional(['RetainedEarningsAccumulatedDeficit']);

  const otherCurrentAssets = currentAssets
    .subtract(cash)
    .subtract(shortTermInvestments)
    .subtract(receivables)
    .subtract(inventory);
  const otherNonCurrentAssets = totalAssets
    .subtract(currentAssets)
    .subtract(ppe)
    .subtract(goodwill)
    .subtract(intangibles);
  const otherCurrentLiabilities = currentLiabilities
    .subtract(accountsPayable)
    .subtract(shortTermDebt);
  const otherNonCurrentLiabilities = totalLiabilities
    .subtract(currentLiabilities)
    .subtract(longTermDebt);
  for (const [label, value] of [
    ['otherCurrentAssets', otherCurrentAssets],
    ['otherNonCurrentAssets', otherNonCurrentAssets],
    ['otherCurrentLiabilities', otherCurrentLiabilities],
    ['otherNonCurrentLiabilities', otherNonCurrentLiabilities],
  ] as const) {
    if (value.isNegative()) {
      throw new ProviderDataError(settings.id, `${label} 计算为负（${period.end}）`);
    }
  }

  return {
    cashAndEquivalents: cash,
    shortTermInvestments,
    accountsReceivable: receivables,
    inventory,
    otherCurrentAssets,
    propertyPlantEquipment: ppe,
    goodwill,
    intangibleAssets: intangibles,
    otherNonCurrentAssets,
    accountsPayable,
    shortTermDebt,
    otherCurrentLiabilities,
    longTermDebt,
    otherNonCurrentLiabilities,
    shareholdersEquity: equity,
    retainedEarnings,
  };
}

/** 读取必需 metric。 */
function requireMetric(
  settings: ResolvedSecOptions,
  index: FactIndex,
  tags: readonly string[],
  unit: string,
  end: string,
  form: string,
  targetDays: number,
  label: string,
): Decimal {
  const value = metric(settings, index, tags, unit, end, form, targetDays);
  if (value === undefined) {
    throw new ProviderDataError(settings.id, `缺少字段 ${label}（${end}）`);
  }
  return value;
}

/** 读取首个可用 metric。 */
function metric(
  settings: ResolvedSecOptions,
  index: FactIndex,
  tags: readonly string[],
  unit: string,
  end: string,
  form: string,
  targetDays: number,
): Decimal | undefined {
  for (const tag of tags) {
    const fact = chooseFact(entriesFor(index, 'us-gaap', tag, unit), end, form, targetDays);
    if (fact !== undefined) return decimalFromFact(fact, settings.id, tag);
  }
  return undefined;
}

/** 读取股本。 */
function requireShares(
  settings: ResolvedSecOptions,
  index: FactIndex,
  end: string,
  form: string,
  targetDays: number,
): Decimal {
  const tags = [
    ['us-gaap', 'WeightedAverageNumberOfDilutedSharesOutstanding'],
    ['us-gaap', 'WeightedAverageNumberOfSharesOutstandingBasic'],
    ['dei', 'EntityCommonStockSharesOutstanding'],
  ] as const;
  for (const [taxonomy, tag] of tags) {
    const fact = chooseFact(entriesFor(index, taxonomy, tag, 'shares'), end, form, targetDays);
    if (fact !== undefined) return decimalFromFact(fact, settings.id, tag);
  }
  throw new ProviderDataError(settings.id, `缺少股本字段（${end}）`);
}

/** 读取某个 tag 的 fact 列表。 */
function entriesFor(
  index: FactIndex,
  taxonomy: string,
  tag: string,
  unit: string,
): readonly SecFact[] {
  const taxonomyValue = index.facts[taxonomy];
  if (!isPlainObject(taxonomyValue)) return [];
  const tagValue = taxonomyValue[tag];
  if (!isPlainObject(tagValue)) return [];
  const units = tagValue['units'];
  if (!isPlainObject(units)) return [];
  const entries = units[unit];
  if (!Array.isArray(entries)) return [];
  return entries.filter(isSecFact);
}

/** 选择匹配 end/form 的 fact。 */
function chooseFact(
  facts: readonly SecFact[],
  end: string,
  form: string,
  targetDays: number,
): SecFact | undefined {
  const candidates = facts.filter(
    (fact) => fact.end === end && fact.form !== undefined && fact.form.startsWith(form),
  );
  if (candidates.length === 0) return undefined;
  let best: SecFact | undefined;
  let bestDuration = Number.POSITIVE_INFINITY;
  let bestFiled = '';
  for (const fact of candidates) {
    const duration =
      fact.start === undefined
        ? Number.POSITIVE_INFINITY
        : Math.abs(daysBetweenIso(fact.start, fact.end) - targetDays);
    const filed = fact.filed ?? '';
    if (duration < bestDuration || (duration === bestDuration && filed > bestFiled)) {
      best = fact;
      bestDuration = duration;
      bestFiled = filed;
    }
  }
  return best;
}

/** 判断 SecFact。 */
function isSecFact(value: unknown): value is SecFact {
  return (
    isPlainObject(value) &&
    typeof value['end'] === 'string' &&
    (typeof value['val'] === 'number' || typeof value['val'] === 'string')
  );
}

/** fact 转 Decimal。 */
function decimalFromFact(fact: SecFact, providerId: string, label: string): Decimal {
  try {
    return Decimal.parse(String(fact.val));
  } catch (cause) {
    throw new ProviderDataError(providerId, `${label} 数值非法: ${String(fact.val)}`, cause);
  }
}

/** 取非负。 */
function maxZero(value: Decimal): Decimal {
  return value.isNegative() ? Decimal.ZERO : value;
}

/** 日期 YYYY-MM-DD 转 UTC 毫秒。 */
function parseDay(value: string, providerId: string): number {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (match === null) throw new ProviderDataError(providerId, `日期非法: ${value}`);
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
    throw new ProviderDataError(providerId, `日期非法: ${value}`);
  }
  return timestamp;
}

/** ISO 日期加减天数。 */
function shiftDate(value: string, days: number): string {
  return new Date(parseDay(value, 'sec-edgar') + days * 86_400_000).toISOString().slice(0, 10);
}

/** 两个 ISO 日期之间的天数。 */
function daysBetweenIso(start: string, end: string): number {
  return Math.round((parseDay(end, 'sec-edgar') - parseDay(start, 'sec-edgar')) / 86_400_000);
}
