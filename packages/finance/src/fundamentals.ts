/**
 * @file 股票/企业基本面：三大财务报表、财务比率、成长性、DuPont、估值倍数与 Piotroski F-score。
 * @description 报表合计行由原始行项目推导；资产恒等式必须精确成立；比率分母不成立时返回 undefined。
 * @module finance/fundamentals
 */

import { Decimal, sumDecimals } from '@kurumi/math';
import { FinanceDefinitionError } from './errors.js';
import { isInstrument, type Instrument } from './instrument.js';
import { createMoney, isCurrency, type Currency, type Money } from './money.js';
import {
  assertDecimal,
  assertNonEmptyString,
  assertOneOf,
  assertPositiveDecimal,
  assertSafeInteger,
  assertTimestampMs,
} from './validation.js';

/** 报表期间类型。 */
export const StatementPeriodType = {
  /** 年报。 */
  Annual: 'annual',
  /** 季报。 */
  Quarterly: 'quarterly',
  /** 滚动十二个月。 */
  Ttm: 'ttm',
} as const;

/** 报表期间类型联合。 */
export type StatementPeriodType = (typeof StatementPeriodType)[keyof typeof StatementPeriodType];

/** 全部期间类型。 */
const PERIOD_TYPES: readonly StatementPeriodType[] = Object.freeze(
  Object.values(StatementPeriodType),
);

/** 财务期间。 */
export interface FiscalPeriod {
  /** 财年，>= 1900。 */
  readonly fiscalYear: number;
  /** 期间类型。 */
  readonly periodType: StatementPeriodType;
  /** 期间开始（epoch 毫秒）。 */
  readonly startMs: number;
  /** 期间结束（epoch 毫秒）。 */
  readonly endMs: number;
  /** 报表货币。 */
  readonly currency: Currency;
}

/** 公司资料。 */
export interface CompanyProfile {
  /** 对应标的。 */
  readonly instrument: Instrument;
  /** 公司名称。 */
  readonly name: string;
  /** 行业板块。 */
  readonly sector?: string;
  /** 细分行业。 */
  readonly industry?: string;
  /** 注册国家/地区代码。 */
  readonly country?: string;
  /** 报表货币。 */
  readonly currency: Currency;
  /** 流通/总股本。 */
  readonly sharesOutstanding?: Decimal;
  /** 公司简介。 */
  readonly description?: string;
}

/** 利润表输入（原始行项目）。 */
export interface IncomeStatementInput {
  /** 营业收入，>= 0。 */
  readonly revenue: Decimal;
  /** 营业成本，>= 0。 */
  readonly costOfRevenue: Decimal;
  /** 营业费用，>= 0。 */
  readonly operatingExpenses: Decimal;
  /** 利息费用，>= 0，缺省 0。 */
  readonly interestExpense?: Decimal;
  /** 其他收入/支出，可为负，缺省 0。 */
  readonly otherIncome?: Decimal;
  /** 所得税费用，>= 0。 */
  readonly taxExpense: Decimal;
  /** 加权平均股本，> 0。 */
  readonly sharesOutstanding: Decimal;
  /** 基本 EPS；缺省由净利润/股本计算。 */
  readonly epsBasic?: Decimal;
  /** 稀释 EPS；缺省沿用基本 EPS 或由净利润/股本计算。 */
  readonly epsDiluted?: Decimal;
}

/** 利润表输出（含推导合计）。 */
export interface IncomeStatement {
  /** 营业收入。 */
  readonly revenue: Decimal;
  /** 营业成本。 */
  readonly costOfRevenue: Decimal;
  /** 毛利润。 */
  readonly grossProfit: Decimal;
  /** 营业费用。 */
  readonly operatingExpenses: Decimal;
  /** 营业利润。 */
  readonly operatingIncome: Decimal;
  /** 利息费用。 */
  readonly interestExpense: Decimal;
  /** 其他收入/支出。 */
  readonly otherIncome: Decimal;
  /** 税前利润。 */
  readonly pretaxIncome: Decimal;
  /** 所得税费用。 */
  readonly taxExpense: Decimal;
  /** 净利润。 */
  readonly netIncome: Decimal;
  /** 基本 EPS。 */
  readonly epsBasic: Decimal;
  /** 稀释 EPS。 */
  readonly epsDiluted: Decimal;
  /** 加权平均股本。 */
  readonly sharesOutstanding: Decimal;
}

/** 资产负债表输入（原始行项目）。 */
export interface BalanceSheetInput {
  /** 现金及等价物，>= 0。 */
  readonly cashAndEquivalents: Decimal;
  /** 短期投资，>= 0。 */
  readonly shortTermInvestments: Decimal;
  /** 应收账款，>= 0。 */
  readonly accountsReceivable: Decimal;
  /** 存货，>= 0。 */
  readonly inventory: Decimal;
  /** 其他流动资产，>= 0。 */
  readonly otherCurrentAssets: Decimal;
  /** 固定资产，>= 0。 */
  readonly propertyPlantEquipment: Decimal;
  /** 商誉，>= 0。 */
  readonly goodwill: Decimal;
  /** 无形资产，>= 0。 */
  readonly intangibleAssets: Decimal;
  /** 其他非流动资产，>= 0。 */
  readonly otherNonCurrentAssets: Decimal;
  /** 应付账款，>= 0。 */
  readonly accountsPayable: Decimal;
  /** 短期债务，>= 0。 */
  readonly shortTermDebt: Decimal;
  /** 其他流动负债，>= 0。 */
  readonly otherCurrentLiabilities: Decimal;
  /** 长期债务，>= 0。 */
  readonly longTermDebt: Decimal;
  /** 其他非流动负债，>= 0。 */
  readonly otherNonCurrentLiabilities: Decimal;
  /** 所有者权益，可为负。 */
  readonly shareholdersEquity: Decimal;
  /** 留存收益，可为负，缺省 0。 */
  readonly retainedEarnings?: Decimal;
}

/** 资产负债表输出（含推导合计）。 */
export interface BalanceSheet {
  /** 现金及等价物。 */
  readonly cashAndEquivalents: Decimal;
  /** 短期投资。 */
  readonly shortTermInvestments: Decimal;
  /** 应收账款。 */
  readonly accountsReceivable: Decimal;
  /** 存货。 */
  readonly inventory: Decimal;
  /** 其他流动资产。 */
  readonly otherCurrentAssets: Decimal;
  /** 流动资产合计。 */
  readonly currentAssets: Decimal;
  /** 固定资产。 */
  readonly propertyPlantEquipment: Decimal;
  /** 商誉。 */
  readonly goodwill: Decimal;
  /** 无形资产。 */
  readonly intangibleAssets: Decimal;
  /** 其他非流动资产。 */
  readonly otherNonCurrentAssets: Decimal;
  /** 资产总计。 */
  readonly totalAssets: Decimal;
  /** 应付账款。 */
  readonly accountsPayable: Decimal;
  /** 短期债务。 */
  readonly shortTermDebt: Decimal;
  /** 其他流动负债。 */
  readonly otherCurrentLiabilities: Decimal;
  /** 流动负债合计。 */
  readonly currentLiabilities: Decimal;
  /** 长期债务。 */
  readonly longTermDebt: Decimal;
  /** 其他非流动负债。 */
  readonly otherNonCurrentLiabilities: Decimal;
  /** 负债合计。 */
  readonly totalLiabilities: Decimal;
  /** 债务合计。 */
  readonly totalDebt: Decimal;
  /** 所有者权益。 */
  readonly shareholdersEquity: Decimal;
  /** 留存收益。 */
  readonly retainedEarnings: Decimal;
  /** 负债与权益合计。 */
  readonly totalLiabilitiesAndEquity: Decimal;
}

/** 现金流量表输入（原始行项目）。 */
export interface CashFlowStatementInput {
  /** 经营活动现金流，可为负。 */
  readonly operatingCashFlow: Decimal;
  /** 资本开支，>= 0。 */
  readonly capitalExpenditure: Decimal;
  /** 投资活动现金流，可为负。 */
  readonly investingCashFlow: Decimal;
  /** 融资活动现金流，可为负。 */
  readonly financingCashFlow: Decimal;
  /** 折旧与摊销，>= 0。 */
  readonly depreciationAmortization: Decimal;
  /** 支付的现金股利，>= 0，缺省 0。 */
  readonly dividendsPaid?: Decimal;
  /** 股份回购金额，>= 0，缺省 0。 */
  readonly shareRepurchases?: Decimal;
}

/** 现金流量表输出（含推导合计）。 */
export interface CashFlowStatement {
  /** 经营活动现金流。 */
  readonly operatingCashFlow: Decimal;
  /** 资本开支。 */
  readonly capitalExpenditure: Decimal;
  /** 自由现金流。 */
  readonly freeCashFlow: Decimal;
  /** 投资活动现金流。 */
  readonly investingCashFlow: Decimal;
  /** 融资活动现金流。 */
  readonly financingCashFlow: Decimal;
  /** 现金净变化。 */
  readonly netChangeInCash: Decimal;
  /** 折旧与摊销。 */
  readonly depreciationAmortization: Decimal;
  /** 支付的现金股利。 */
  readonly dividendsPaid: Decimal;
  /** 股份回购金额。 */
  readonly shareRepurchases: Decimal;
}

/** 财务报表集合。 */
export interface FinancialStatements {
  /** 期间。 */
  readonly period: FiscalPeriod;
  /** 利润表。 */
  readonly income: IncomeStatement;
  /** 资产负债表。 */
  readonly balance: BalanceSheet;
  /** 现金流量表。 */
  readonly cashFlow: CashFlowStatement;
}

/** 财务报表创建输入。 */
export interface CreateFinancialStatementsInput {
  /** 期间。 */
  readonly period: FiscalPeriod;
  /** 利润表原始行项目。 */
  readonly income: IncomeStatementInput;
  /** 资产负债表原始行项目。 */
  readonly balance: BalanceSheetInput;
  /** 现金流量表原始行项目。 */
  readonly cashFlow: CashFlowStatementInput;
}

/** 核心财务比率。 */
export interface FundamentalRatios {
  /** 流动比率。 */
  readonly currentRatio?: Decimal;
  /** 速动比率。 */
  readonly quickRatio?: Decimal;
  /** 现金比率。 */
  readonly cashRatio?: Decimal;
  /** 营运资本。 */
  readonly workingCapital?: Money;
  /** 债务/权益。 */
  readonly debtToEquity?: Decimal;
  /** 债务/资产。 */
  readonly debtToAssets?: Decimal;
  /** 利息保障倍数。 */
  readonly interestCoverage?: Decimal;
  /** 毛利率。 */
  readonly grossMargin?: Decimal;
  /** 营业利润率。 */
  readonly operatingMargin?: Decimal;
  /** 净利率。 */
  readonly netMargin?: Decimal;
  /** 总资产收益率。 */
  readonly returnOnAssets?: Decimal;
  /** 净资产收益率。 */
  readonly returnOnEquity?: Decimal;
  /** 投入资本回报率。 */
  readonly returnOnInvestedCapital?: Decimal;
  /** 已动用资本回报率。 */
  readonly returnOnCapitalEmployed?: Decimal;
  /** 总资产周转率。 */
  readonly assetTurnover?: Decimal;
  /** 存货周转率。 */
  readonly inventoryTurnover?: Decimal;
  /** 应收账款周转率。 */
  readonly receivablesTurnover?: Decimal;
  /** 存货周转天数。 */
  readonly daysInventoryOutstanding?: Decimal;
  /** 应收账款周转天数。 */
  readonly daysSalesOutstanding?: Decimal;
  /** 自由现金流利润率。 */
  readonly freeCashFlowMargin?: Decimal;
  /** 稀释 EPS。 */
  readonly earningsPerShare?: Decimal;
  /** 每股净资产。 */
  readonly bookValuePerShare?: Decimal;
  /** 每股自由现金流。 */
  readonly freeCashFlowPerShare?: Decimal;
  /** 股利支付率。 */
  readonly dividendPayoutRatio?: Decimal;
  /** 有效税率。 */
  readonly effectiveTaxRate?: Decimal;
}

/** 成长性指标。 */
export interface GrowthMetrics {
  /** 收入增长率。 */
  readonly revenueGrowth?: Decimal;
  /** 净利润增长率。 */
  readonly netIncomeGrowth?: Decimal;
  /** EPS 增长率。 */
  readonly epsGrowth?: Decimal;
  /** 自由现金流增长率。 */
  readonly freeCashFlowGrowth?: Decimal;
  /** 权益增长率。 */
  readonly equityGrowth?: Decimal;
}

/** DuPont 分解。 */
export interface DuPontAnalysis {
  /** 净利率。 */
  readonly netMargin?: Decimal;
  /** 总资产周转率。 */
  readonly assetTurnover?: Decimal;
  /** 权益乘数。 */
  readonly equityMultiplier?: Decimal;
  /** 三因子乘积/Direct ROE。 */
  readonly returnOnEquity?: Decimal;
  /** 税收负担（净利润/税前利润）。 */
  readonly taxBurden?: Decimal;
  /** 利息负担（税前利润/营业利润）。 */
  readonly interestBurden?: Decimal;
  /** 营业利润率。 */
  readonly operatingMargin?: Decimal;
  /** 五因子乘积。 */
  readonly fiveFactorRoe?: Decimal;
}

/** 估值输入。 */
export interface ValuationInput {
  /** 每股价格，> 0。 */
  readonly price: Decimal;
  /** 股本；缺省使用利润表股本。 */
  readonly sharesOutstanding?: Decimal;
  /** 预期增长率（0.15 表示 15%），用于 PEG。 */
  readonly growthRate?: Decimal;
}

/** 相对估值倍数。 */
export interface ValuationMultiples {
  /** 市值。 */
  readonly marketCap?: Money;
  /** 企业价值。 */
  readonly enterpriseValue?: Money;
  /** 市盈率。 */
  readonly peRatio?: Decimal;
  /** 市净率。 */
  readonly pbRatio?: Decimal;
  /** 市销率。 */
  readonly psRatio?: Decimal;
  /** EV/EBITDA。 */
  readonly evToEbitda?: Decimal;
  /** EV/Sales。 */
  readonly evToSales?: Decimal;
  /** 股息率。 */
  readonly dividendYield?: Decimal;
  /** 盈利收益率。 */
  readonly earningsYield?: Decimal;
  /** PEG。 */
  readonly pegRatio?: Decimal;
}

/** Piotroski 准则。 */
export const PiotroskiCriterion = {
  PositiveNetIncome: 'positive_net_income',
  PositiveOperatingCashFlow: 'positive_operating_cash_flow',
  ImprovingRoa: 'improving_roa',
  OperatingCashFlowExceedsNetIncome: 'operating_cash_flow_exceeds_net_income',
  DecreasingLeverage: 'decreasing_leverage',
  ImprovingCurrentRatio: 'improving_current_ratio',
  NoDilution: 'no_dilution',
  ImprovingGrossMargin: 'improving_gross_margin',
  ImprovingAssetTurnover: 'improving_asset_turnover',
} as const;

/** Piotroski 准则联合。 */
export type PiotroskiCriterion = (typeof PiotroskiCriterion)[keyof typeof PiotroskiCriterion];

/** Piotroski 结果。 */
export interface PiotroskiResult {
  /** 0..9 分。 */
  readonly score: number;
  /** 9 项准则命中情况。 */
  readonly criteria: Readonly<Record<PiotroskiCriterion, boolean>>;
}

/**
 * 创建财务期间。
 *
 * @param period - 财年、期间类型、起止时间与货币
 * @returns 冻结期间
 * @throws {FinanceDefinitionError} 字段非法
 */
export function createFiscalPeriod(period: FiscalPeriod): FiscalPeriod {
  assertSafeInteger(period.fiscalYear, 'period.fiscalYear');
  if (period.fiscalYear < 1900) {
    throw new FinanceDefinitionError(
      `period.fiscalYear 必须 >= 1900（实际值: ${period.fiscalYear}）`,
    );
  }
  assertOneOf(period.periodType, PERIOD_TYPES, 'period.periodType');
  assertTimestampMs(period.startMs, 'period.startMs');
  assertTimestampMs(period.endMs, 'period.endMs');
  if (period.endMs <= period.startMs) {
    throw new FinanceDefinitionError(
      `period.endMs 必须晚于 startMs（${period.startMs} -> ${period.endMs}）`,
    );
  }
  if (!isCurrency(period.currency)) {
    throw new FinanceDefinitionError('period.currency 非法');
  }
  return Object.freeze({
    fiscalYear: period.fiscalYear,
    periodType: period.periodType,
    startMs: period.startMs,
    endMs: period.endMs,
    currency: period.currency,
  });
}

/**
 * 创建公司资料。
 *
 * @param profile - 标的、名称与可选元数据
 * @returns 冻结资料
 * @throws {FinanceDefinitionError} 字段非法
 */
export function createCompanyProfile(profile: CompanyProfile): CompanyProfile {
  if (!isInstrument(profile.instrument)) {
    throw new FinanceDefinitionError('company.instrument 非法');
  }
  assertNonEmptyString(profile.name, 'company.name');
  if (!isCurrency(profile.currency)) {
    throw new FinanceDefinitionError('company.currency 非法');
  }
  if (profile.sharesOutstanding !== undefined) {
    assertPositiveDecimal(profile.sharesOutstanding, 'company.sharesOutstanding');
  }
  for (const [key, value] of [
    ['sector', profile.sector],
    ['industry', profile.industry],
    ['country', profile.country],
    ['description', profile.description],
  ] as const) {
    if (value !== undefined) assertNonEmptyString(value, `company.${key}`);
  }
  return Object.freeze({
    instrument: profile.instrument,
    name: profile.name,
    currency: profile.currency,
    ...(profile.sector === undefined ? {} : { sector: profile.sector }),
    ...(profile.industry === undefined ? {} : { industry: profile.industry }),
    ...(profile.country === undefined ? {} : { country: profile.country }),
    ...(profile.sharesOutstanding === undefined
      ? {}
      : { sharesOutstanding: profile.sharesOutstanding }),
    ...(profile.description === undefined ? {} : { description: profile.description }),
  });
}

/**
 * 创建财务报表集合。
 *
 * @param input - 期间与三大报表原始行项目
 * @returns 冻结报表；合计行由行项目推导
 * @throws {FinanceDefinitionError} 行项目非法或资产恒等式不成立
 *
 * @example
 * ```ts
 * const statements = createFinancialStatements({
 *   period: { fiscalYear: 2025, periodType: 'annual', startMs: 1, endMs: 2, currency: usd },
 *   income: { revenue, costOfRevenue, operatingExpenses, taxExpense, sharesOutstanding },
 *   balance: { ...lineItems, shareholdersEquity },
 *   cashFlow: { operatingCashFlow, capitalExpenditure, investingCashFlow,
 *     financingCashFlow, depreciationAmortization },
 * });
 * ```
 */
export function createFinancialStatements(
  input: CreateFinancialStatementsInput,
): FinancialStatements {
  const period = createFiscalPeriod(input.period);
  const income = createIncomeStatement(input.income);
  const balance = createBalanceSheet(input.balance);
  const cashFlow = createCashFlowStatement(input.cashFlow);
  return Object.freeze({ period, income, balance, cashFlow });
}

/** 创建利润表。 */
function createIncomeStatement(input: IncomeStatementInput): IncomeStatement {
  assertNonNegative(input.revenue, 'income.revenue');
  assertNonNegative(input.costOfRevenue, 'income.costOfRevenue');
  assertNonNegative(input.operatingExpenses, 'income.operatingExpenses');
  const interestExpense = input.interestExpense ?? Decimal.ZERO;
  const otherIncome = input.otherIncome ?? Decimal.ZERO;
  assertNonNegative(interestExpense, 'income.interestExpense');
  assertDecimal(otherIncome, 'income.otherIncome');
  assertNonNegative(input.taxExpense, 'income.taxExpense');
  assertPositiveDecimal(input.sharesOutstanding, 'income.sharesOutstanding');

  const grossProfit = input.revenue.subtract(input.costOfRevenue);
  const operatingIncome = grossProfit.subtract(input.operatingExpenses);
  const pretaxIncome = operatingIncome.add(otherIncome).subtract(interestExpense);
  const netIncome = pretaxIncome.subtract(input.taxExpense);
  const computedEps = netIncome.divide(input.sharesOutstanding, {
    scale: 8,
    rounding: 'half-even',
  });
  const epsBasic = input.epsBasic ?? input.epsDiluted ?? computedEps;
  const epsDiluted = input.epsDiluted ?? epsBasic;
  assertDecimal(epsBasic, 'income.epsBasic');
  assertDecimal(epsDiluted, 'income.epsDiluted');

  return Object.freeze({
    revenue: input.revenue,
    costOfRevenue: input.costOfRevenue,
    grossProfit,
    operatingExpenses: input.operatingExpenses,
    operatingIncome,
    interestExpense,
    otherIncome,
    pretaxIncome,
    taxExpense: input.taxExpense,
    netIncome,
    epsBasic,
    epsDiluted,
    sharesOutstanding: input.sharesOutstanding,
  });
}

/** 创建资产负债表。 */
function createBalanceSheet(input: BalanceSheetInput): BalanceSheet {
  const assetKeys = [
    'cashAndEquivalents',
    'shortTermInvestments',
    'accountsReceivable',
    'inventory',
    'otherCurrentAssets',
    'propertyPlantEquipment',
    'goodwill',
    'intangibleAssets',
    'otherNonCurrentAssets',
  ] as const;
  for (const key of assetKeys) assertNonNegative(input[key], `balance.${key}`);
  const liabilityKeys = [
    'accountsPayable',
    'shortTermDebt',
    'otherCurrentLiabilities',
    'longTermDebt',
    'otherNonCurrentLiabilities',
  ] as const;
  for (const key of liabilityKeys) assertNonNegative(input[key], `balance.${key}`);
  assertDecimal(input.shareholdersEquity, 'balance.shareholdersEquity');
  const retainedEarnings = input.retainedEarnings ?? Decimal.ZERO;
  assertDecimal(retainedEarnings, 'balance.retainedEarnings');

  const currentAssets = sumDecimals([
    input.cashAndEquivalents,
    input.shortTermInvestments,
    input.accountsReceivable,
    input.inventory,
    input.otherCurrentAssets,
  ]);
  const totalAssets = sumDecimals([
    currentAssets,
    input.propertyPlantEquipment,
    input.goodwill,
    input.intangibleAssets,
    input.otherNonCurrentAssets,
  ]);
  const currentLiabilities = sumDecimals([
    input.accountsPayable,
    input.shortTermDebt,
    input.otherCurrentLiabilities,
  ]);
  const totalLiabilities = sumDecimals([
    currentLiabilities,
    input.longTermDebt,
    input.otherNonCurrentLiabilities,
  ]);
  const totalDebt = input.shortTermDebt.add(input.longTermDebt);
  const totalLiabilitiesAndEquity = totalLiabilities.add(input.shareholdersEquity);
  if (!totalAssets.equals(totalLiabilitiesAndEquity)) {
    throw new FinanceDefinitionError(
      `资产负债表不成立：assets ${totalAssets.toString()} != liabilities + equity ${totalLiabilitiesAndEquity.toString()}（差额 ${totalAssets.subtract(totalLiabilitiesAndEquity).toString()}）`,
    );
  }

  return Object.freeze({
    cashAndEquivalents: input.cashAndEquivalents,
    shortTermInvestments: input.shortTermInvestments,
    accountsReceivable: input.accountsReceivable,
    inventory: input.inventory,
    otherCurrentAssets: input.otherCurrentAssets,
    currentAssets,
    propertyPlantEquipment: input.propertyPlantEquipment,
    goodwill: input.goodwill,
    intangibleAssets: input.intangibleAssets,
    otherNonCurrentAssets: input.otherNonCurrentAssets,
    totalAssets,
    accountsPayable: input.accountsPayable,
    shortTermDebt: input.shortTermDebt,
    otherCurrentLiabilities: input.otherCurrentLiabilities,
    currentLiabilities,
    longTermDebt: input.longTermDebt,
    otherNonCurrentLiabilities: input.otherNonCurrentLiabilities,
    totalLiabilities,
    totalDebt,
    shareholdersEquity: input.shareholdersEquity,
    retainedEarnings,
    totalLiabilitiesAndEquity,
  });
}

/** 创建现金流量表。 */
function createCashFlowStatement(input: CashFlowStatementInput): CashFlowStatement {
  assertDecimal(input.operatingCashFlow, 'cashFlow.operatingCashFlow');
  assertNonNegative(input.capitalExpenditure, 'cashFlow.capitalExpenditure');
  assertDecimal(input.investingCashFlow, 'cashFlow.investingCashFlow');
  assertDecimal(input.financingCashFlow, 'cashFlow.financingCashFlow');
  assertNonNegative(input.depreciationAmortization, 'cashFlow.depreciationAmortization');
  const dividendsPaid = input.dividendsPaid ?? Decimal.ZERO;
  const shareRepurchases = input.shareRepurchases ?? Decimal.ZERO;
  assertNonNegative(dividendsPaid, 'cashFlow.dividendsPaid');
  assertNonNegative(shareRepurchases, 'cashFlow.shareRepurchases');

  return Object.freeze({
    operatingCashFlow: input.operatingCashFlow,
    capitalExpenditure: input.capitalExpenditure,
    freeCashFlow: input.operatingCashFlow.subtract(input.capitalExpenditure),
    investingCashFlow: input.investingCashFlow,
    financingCashFlow: input.financingCashFlow,
    netChangeInCash: input.operatingCashFlow
      .add(input.investingCashFlow)
      .add(input.financingCashFlow),
    depreciationAmortization: input.depreciationAmortization,
    dividendsPaid,
    shareRepurchases,
  });
}

/**
 * 计算核心财务比率。
 *
 * @param statements - 当期报表
 * @param prior - 可选上一期；提供时 ROA/ROE/周转率使用平均余额
 * @returns 冻结比率对象；分母不成立时字段缺省
 * @throws {FinanceDefinitionError} 期间不可比
 */
export function computeFundamentalRatios(
  statements: FinancialStatements,
  prior?: FinancialStatements,
): FundamentalRatios {
  if (prior !== undefined) assertComparable(statements, prior);
  const { income, balance, cashFlow } = statements;
  const averageAssets = averageDecimal(balance.totalAssets, prior?.balance.totalAssets);
  const averageEquity = averageDecimal(
    balance.shareholdersEquity,
    prior?.balance.shareholdersEquity,
  );
  const revenue = income.revenue;
  const netIncome = income.netIncome;
  const taxRate = divideOptional(income.taxExpense, income.pretaxIncome);
  const nopat = income.operatingIncome.multiply(Decimal.ONE.subtract(taxRate ?? Decimal.ZERO));
  const investedCapital = balance.totalDebt
    .add(balance.shareholdersEquity)
    .subtract(balance.cashAndEquivalents)
    .subtract(balance.shortTermInvestments);
  const capitalEmployed = balance.totalAssets.subtract(balance.currentLiabilities);
  const inventoryTurnover = divideOptional(income.costOfRevenue, balance.inventory);
  const receivablesTurnover = divideOptional(revenue, balance.accountsReceivable);
  const daysInventoryOutstanding = divideOptional(
    Decimal.fromInteger(365).multiply(balance.inventory),
    income.costOfRevenue,
  );
  const daysSalesOutstanding = divideOptional(
    Decimal.fromInteger(365).multiply(balance.accountsReceivable),
    revenue,
  );
  const bookValuePerShare = divideOptional(balance.shareholdersEquity, income.sharesOutstanding);

  return Object.freeze({
    ...optional('currentRatio', divideOptional(balance.currentAssets, balance.currentLiabilities)),
    ...optional(
      'quickRatio',
      divideOptional(balance.currentAssets.subtract(balance.inventory), balance.currentLiabilities),
    ),
    ...optional(
      'cashRatio',
      divideOptional(
        balance.cashAndEquivalents.add(balance.shortTermInvestments),
        balance.currentLiabilities,
      ),
    ),
    ...(balance.currentLiabilities.isPositive()
      ? {
          workingCapital: createMoney(
            balance.currentAssets.subtract(balance.currentLiabilities),
            statements.period.currency,
          ),
        }
      : {}),
    ...optional('debtToEquity', divideOptional(balance.totalDebt, balance.shareholdersEquity)),
    ...optional('debtToAssets', divideOptional(balance.totalDebt, balance.totalAssets)),
    ...optional('interestCoverage', divideOptional(income.operatingIncome, income.interestExpense)),
    ...optional('grossMargin', divideOptional(income.grossProfit, revenue)),
    ...optional('operatingMargin', divideOptional(income.operatingIncome, revenue)),
    ...optional('netMargin', divideOptional(netIncome, revenue)),
    ...optional('returnOnAssets', divideOptional(netIncome, averageAssets)),
    ...optional('returnOnEquity', divideOptional(netIncome, averageEquity)),
    ...optional('returnOnInvestedCapital', divideOptional(nopat, investedCapital)),
    ...optional('returnOnCapitalEmployed', divideOptional(income.operatingIncome, capitalEmployed)),
    ...optional('assetTurnover', divideOptional(revenue, averageAssets)),
    ...optional('inventoryTurnover', inventoryTurnover),
    ...optional('receivablesTurnover', receivablesTurnover),
    ...optional('daysInventoryOutstanding', daysInventoryOutstanding),
    ...optional('daysSalesOutstanding', daysSalesOutstanding),
    ...optional('freeCashFlowMargin', divideOptional(cashFlow.freeCashFlow, revenue)),
    ...optional('earningsPerShare', income.epsDiluted),
    ...optional(
      'bookValuePerShare',
      balance.shareholdersEquity.isPositive() ? bookValuePerShare : undefined,
    ),
    ...optional(
      'freeCashFlowPerShare',
      divideOptional(cashFlow.freeCashFlow, income.sharesOutstanding),
    ),
    ...optional('dividendPayoutRatio', divideOptional(cashFlow.dividendsPaid, netIncome)),
    ...optional('effectiveTaxRate', taxRate),
  });
}

/**
 * 计算同比/环比成长性。
 *
 * @param current - 当期报表
 * @param prior - 上一期报表；必须同币种、同期间类型
 * @returns 冻结成长指标；上期分母不成立时字段缺省
 * @throws {FinanceDefinitionError} 期间不可比
 */
export function computeGrowthMetrics(
  current: FinancialStatements,
  prior: FinancialStatements,
): GrowthMetrics {
  assertComparable(current, prior);
  return Object.freeze({
    ...optional('revenueGrowth', growthRate(current.income.revenue, prior.income.revenue)),
    ...optional('netIncomeGrowth', growthRate(current.income.netIncome, prior.income.netIncome)),
    ...optional('epsGrowth', growthRate(current.income.epsDiluted, prior.income.epsDiluted)),
    ...optional(
      'freeCashFlowGrowth',
      growthRate(current.cashFlow.freeCashFlow, prior.cashFlow.freeCashFlow),
    ),
    ...optional(
      'equityGrowth',
      growthRate(current.balance.shareholdersEquity, prior.balance.shareholdersEquity),
    ),
  });
}

/**
 * DuPont 分解。
 *
 * @param statements - 当期报表
 * @param prior - 可选上一期；用于平均余额
 * @returns 三因子与五因子分解；因子不成立时字段缺省
 */
export function computeDuPont(
  statements: FinancialStatements,
  prior?: FinancialStatements,
): DuPontAnalysis {
  if (prior !== undefined) assertComparable(statements, prior);
  const { income, balance } = statements;
  const averageAssets = averageDecimal(balance.totalAssets, prior?.balance.totalAssets);
  const averageEquity = averageDecimal(
    balance.shareholdersEquity,
    prior?.balance.shareholdersEquity,
  );
  const netMargin = divideOptional(income.netIncome, income.revenue);
  const assetTurnover = divideOptional(income.revenue, averageAssets);
  const equityMultiplier = divideOptional(averageAssets, averageEquity);
  const taxBurden = divideOptional(income.netIncome, income.pretaxIncome);
  const interestBurden = divideOptional(income.pretaxIncome, income.operatingIncome);
  const operatingMargin = divideOptional(income.operatingIncome, income.revenue);
  const directRoe = divideOptional(income.netIncome, averageEquity);

  // 每个因子均为 scale 8；显式指定中间 scale，避免自然 scale 累积后超过 Decimal 上限 30。
  const fiveFactorRoe =
    taxBurden !== undefined &&
    interestBurden !== undefined &&
    operatingMargin !== undefined &&
    assetTurnover !== undefined &&
    equityMultiplier !== undefined
      ? taxBurden
          .multiply(interestBurden, { scale: 8, rounding: 'half-even' })
          .multiply(operatingMargin, { scale: 8, rounding: 'half-even' })
          .multiply(assetTurnover, { scale: 8, rounding: 'half-even' })
          .multiply(equityMultiplier, { scale: 8, rounding: 'half-even' })
      : undefined;

  return Object.freeze({
    ...optional('netMargin', netMargin),
    ...optional('assetTurnover', assetTurnover),
    ...optional('equityMultiplier', equityMultiplier),
    ...optional('returnOnEquity', directRoe),
    ...optional('taxBurden', taxBurden),
    ...optional('interestBurden', interestBurden),
    ...optional('operatingMargin', operatingMargin),
    ...optional('fiveFactorRoe', fiveFactorRoe),
  });
}

/**
 * 计算相对估值倍数。
 *
 * @param statements - 当期报表
 * @param input - 每股价格、可选股本与增长率
 * @returns 冻结倍数；分母不成立时字段缺省
 * @throws {FinanceDefinitionError} 价格/股本非法
 */
export function computeValuationMultiples(
  statements: FinancialStatements,
  input: ValuationInput,
): ValuationMultiples {
  assertPositiveDecimal(input.price, 'valuation.price');
  const shares = input.sharesOutstanding ?? statements.income.sharesOutstanding;
  assertPositiveDecimal(shares, 'valuation.sharesOutstanding');
  if (input.growthRate !== undefined) {
    assertDecimal(input.growthRate, 'valuation.growthRate');
  }

  const currency = statements.period.currency;
  const marketCapAmount = input.price.multiply(shares);
  const marketCap = createMoney(marketCapAmount, currency);
  const netDebt = statements.balance.totalDebt
    .subtract(statements.balance.cashAndEquivalents)
    .subtract(statements.balance.shortTermInvestments);
  const enterpriseValue = createMoney(marketCapAmount.add(netDebt), currency);
  const ebitda = statements.income.operatingIncome.add(
    statements.cashFlow.depreciationAmortization,
  );
  const peRatio = divideOptional(marketCapAmount, statements.income.netIncome);
  const pegRatio =
    peRatio !== undefined && input.growthRate !== undefined && input.growthRate.isPositive()
      ? peRatio.divide(input.growthRate.multiply(Decimal.fromInteger(100)), {
          scale: 8,
          rounding: 'half-even',
        })
      : undefined;

  return Object.freeze({
    marketCap,
    enterpriseValue,
    ...optional('peRatio', peRatio),
    ...optional('pbRatio', divideOptional(marketCapAmount, statements.balance.shareholdersEquity)),
    ...optional('psRatio', divideOptional(marketCapAmount, statements.income.revenue)),
    ...optional('evToEbitda', divideOptional(enterpriseValue.amount, ebitda)),
    ...optional('evToSales', divideOptional(enterpriseValue.amount, statements.income.revenue)),
    ...optional(
      'dividendYield',
      divideOptional(statements.cashFlow.dividendsPaid, marketCapAmount),
    ),
    ...optional('earningsYield', divideOptional(statements.income.netIncome, marketCapAmount)),
    ...optional('pegRatio', pegRatio),
  });
}

/**
 * 计算 Piotroski F-score。
 *
 * @param current - 当期报表
 * @param prior - 上一期报表
 * @returns 0..9 分与逐项准则
 * @throws {FinanceDefinitionError} 期间不可比
 */
export function computePiotroskiFScore(
  current: FinancialStatements,
  prior: FinancialStatements,
): PiotroskiResult {
  assertComparable(current, prior);
  const currentRoa = divideOptional(current.income.netIncome, current.balance.totalAssets);
  const priorRoa = divideOptional(prior.income.netIncome, prior.balance.totalAssets);
  const currentLeverage = divideOptional(current.balance.longTermDebt, current.balance.totalAssets);
  const priorLeverage = divideOptional(prior.balance.longTermDebt, prior.balance.totalAssets);
  const currentRatio = divideOptional(
    current.balance.currentAssets,
    current.balance.currentLiabilities,
  );
  const priorRatio = divideOptional(prior.balance.currentAssets, prior.balance.currentLiabilities);
  const currentGrossMargin = divideOptional(current.income.grossProfit, current.income.revenue);
  const priorGrossMargin = divideOptional(prior.income.grossProfit, prior.income.revenue);
  const currentTurnover = divideOptional(current.income.revenue, current.balance.totalAssets);
  const priorTurnover = divideOptional(prior.income.revenue, prior.balance.totalAssets);

  const criteria: Record<PiotroskiCriterion, boolean> = {
    [PiotroskiCriterion.PositiveNetIncome]: current.income.netIncome.isPositive(),
    [PiotroskiCriterion.PositiveOperatingCashFlow]: current.cashFlow.operatingCashFlow.isPositive(),
    [PiotroskiCriterion.ImprovingRoa]:
      currentRoa !== undefined && priorRoa !== undefined && currentRoa.compare(priorRoa) > 0,
    [PiotroskiCriterion.OperatingCashFlowExceedsNetIncome]:
      current.cashFlow.operatingCashFlow.compare(current.income.netIncome) > 0,
    [PiotroskiCriterion.DecreasingLeverage]:
      currentLeverage !== undefined &&
      priorLeverage !== undefined &&
      currentLeverage.compare(priorLeverage) < 0,
    [PiotroskiCriterion.ImprovingCurrentRatio]:
      currentRatio !== undefined &&
      priorRatio !== undefined &&
      currentRatio.compare(priorRatio) > 0,
    [PiotroskiCriterion.NoDilution]:
      current.income.sharesOutstanding.compare(prior.income.sharesOutstanding) <= 0,
    [PiotroskiCriterion.ImprovingGrossMargin]:
      currentGrossMargin !== undefined &&
      priorGrossMargin !== undefined &&
      currentGrossMargin.compare(priorGrossMargin) > 0,
    [PiotroskiCriterion.ImprovingAssetTurnover]:
      currentTurnover !== undefined &&
      priorTurnover !== undefined &&
      currentTurnover.compare(priorTurnover) > 0,
  };
  const score = Object.values(criteria).filter(Boolean).length;
  return Object.freeze({ score, criteria: Object.freeze(criteria) });
}

/** 校验非负 Decimal。 */
function assertNonNegative(value: Decimal, path: string): void {
  assertDecimal(value, path);
  if (value.isNegative()) {
    throw new FinanceDefinitionError(`${path} 不能为负（实际值: ${value.toString()}）`);
  }
}

/** 安全除法：分母必须为正，否则返回 undefined。 */
function divideOptional(numerator: Decimal, denominator: Decimal, scale = 8): Decimal | undefined {
  if (!denominator.isPositive()) return undefined;
  return numerator.divide(denominator, { scale, rounding: 'half-even' });
}

/** 平均余额；无上期时使用当期。 */
function averageDecimal(current: Decimal, prior: Decimal | undefined): Decimal {
  return prior === undefined
    ? current
    : current.add(prior).divide(Decimal.fromInteger(2), {
        scale: 8,
        rounding: 'half-even',
      });
}

/** 成长率：上期必须为正。 */
function growthRate(current: Decimal, prior: Decimal): Decimal | undefined {
  if (!prior.isPositive()) return undefined;
  return current.subtract(prior).divide(prior, { scale: 8, rounding: 'half-even' });
}

/** 校验两期可比。 */
function assertComparable(current: FinancialStatements, prior: FinancialStatements): void {
  if (current.period.periodType !== prior.period.periodType) {
    throw new FinanceDefinitionError(
      `报表期间类型不可比（${current.period.periodType} vs ${prior.period.periodType}）`,
    );
  }
  if (
    current.period.currency.code !== prior.period.currency.code ||
    current.period.currency.decimals !== prior.period.currency.decimals
  ) {
    throw new FinanceDefinitionError('报表货币不可比');
  }
  if (current.period.endMs <= prior.period.endMs) {
    throw new FinanceDefinitionError('当期结束时间必须晚于上一期');
  }
}

/** 条件可选字段。 */
function optional<TKey extends string, TValue>(
  key: TKey,
  value: TValue | undefined,
): Partial<Record<TKey, TValue>> {
  return value === undefined ? {} : ({ [key]: value } as Record<TKey, TValue>);
}
