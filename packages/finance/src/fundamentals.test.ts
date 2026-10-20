/**
 * @file 基本面单元测试：报表推导、资产恒等式、比率、成长、DuPont、估值倍数与 Piotroski。
 * @description 覆盖 FC-FUND-* 与 I-FIN-013 / I-FIN-014。
 * @module finance/fundamentals.test
 */

import { describe, expect, it } from 'vitest';

import { Decimal } from '@kurumi/math';
import { FinanceDefinitionError } from './errors.js';
import {
  PiotroskiCriterion,
  StatementPeriodType,
  computeDuPont,
  computeFundamentalRatios,
  computeGrowthMetrics,
  computePiotroskiFScore,
  computeValuationMultiples,
  createCompanyProfile,
  createFinancialStatements,
  createFiscalPeriod,
  type BalanceSheetInput,
  type CashFlowStatementInput,
  type FinancialStatements,
  type IncomeStatementInput,
} from './fundamentals.js';
import { createCurrency, type Currency } from './money.js';
import { T0, USD, aapl } from './fixtures.test-utils.js';

const DAY_MS = 86_400_000;

function d(value: string): Decimal {
  return Decimal.parse(value);
}

function income(overrides: Partial<IncomeStatementInput> = {}): IncomeStatementInput {
  return {
    revenue: d('1000'),
    costOfRevenue: d('600'),
    operatingExpenses: d('200'),
    interestExpense: d('20'),
    taxExpense: d('36'),
    sharesOutstanding: d('100'),
    ...overrides,
  };
}

function balance(overrides: Partial<BalanceSheetInput> = {}): BalanceSheetInput {
  return {
    cashAndEquivalents: d('100'),
    shortTermInvestments: d('50'),
    accountsReceivable: d('150'),
    inventory: d('100'),
    otherCurrentAssets: d('50'),
    propertyPlantEquipment: d('500'),
    goodwill: d('100'),
    intangibleAssets: d('50'),
    otherNonCurrentAssets: d('50'),
    accountsPayable: d('100'),
    shortTermDebt: d('50'),
    otherCurrentLiabilities: d('50'),
    longTermDebt: d('300'),
    otherNonCurrentLiabilities: d('50'),
    shareholdersEquity: d('600'),
    retainedEarnings: d('200'),
    ...overrides,
  };
}

function cashFlow(overrides: Partial<CashFlowStatementInput> = {}): CashFlowStatementInput {
  return {
    operatingCashFlow: d('180'),
    capitalExpenditure: d('50'),
    investingCashFlow: d('-100'),
    financingCashFlow: d('-20'),
    depreciationAmortization: d('40'),
    dividendsPaid: d('30'),
    shareRepurchases: d('10'),
    ...overrides,
  };
}

function statements(
  periodOverrides: {
    fiscalYear?: number;
    periodType?: StatementPeriodType;
    startMs?: number;
    endMs?: number;
    currency?: Currency;
  } = {},
  overrides: {
    income?: Partial<IncomeStatementInput>;
    balance?: Partial<BalanceSheetInput>;
    cashFlow?: Partial<CashFlowStatementInput>;
  } = {},
): FinancialStatements {
  return createFinancialStatements({
    period: {
      fiscalYear: periodOverrides.fiscalYear ?? 2025,
      periodType: periodOverrides.periodType ?? StatementPeriodType.Annual,
      startMs: periodOverrides.startMs ?? T0,
      endMs: periodOverrides.endMs ?? T0 + 365 * DAY_MS,
      currency: periodOverrides.currency ?? USD,
    },
    income: income(overrides.income),
    balance: balance(overrides.balance),
    cashFlow: cashFlow(overrides.cashFlow),
  });
}

function priorStatements(): FinancialStatements {
  return statements(
    { fiscalYear: 2024, startMs: T0 - 365 * DAY_MS, endMs: T0 },
    {
      income: {
        revenue: d('900'),
        costOfRevenue: d('550'),
        operatingExpenses: d('180'),
        taxExpense: d('30'),
      },
      balance: {
        cashAndEquivalents: d('100'),
        shortTermInvestments: d('50'),
        accountsReceivable: d('140'),
        inventory: d('110'),
        otherCurrentAssets: d('0'),
        propertyPlantEquipment: d('500'),
        goodwill: d('100'),
        intangibleAssets: d('50'),
        otherNonCurrentAssets: d('50'),
        accountsPayable: d('110'),
        shortTermDebt: d('50'),
        otherCurrentLiabilities: d('50'),
        longTermDebt: d('320'),
        otherNonCurrentLiabilities: d('50'),
        shareholdersEquity: d('520'),
        retainedEarnings: d('150'),
      },
      cashFlow: {
        operatingCashFlow: d('150'),
        capitalExpenditure: d('40'),
        investingCashFlow: d('-90'),
        financingCashFlow: d('-20'),
        depreciationAmortization: d('35'),
        dividendsPaid: d('20'),
        shareRepurchases: d('0'),
      },
    },
  );
}

describe('createFiscalPeriod / createCompanyProfile', () => {
  it('创建并校验期间', () => {
    const period = createFiscalPeriod({
      fiscalYear: 2025,
      periodType: StatementPeriodType.Annual,
      startMs: T0,
      endMs: T0 + DAY_MS,
      currency: USD,
    });
    expect(period.periodType).toBe('annual');
    expect(Object.isFrozen(period)).toBe(true);
    expect(() =>
      createFiscalPeriod({
        fiscalYear: 1899,
        periodType: StatementPeriodType.Annual,
        startMs: T0,
        endMs: T0 + DAY_MS,
        currency: USD,
      }),
    ).toThrow(FinanceDefinitionError);
    expect(() =>
      createFiscalPeriod({
        fiscalYear: 2025,
        periodType: 'monthly' as never,
        startMs: T0,
        endMs: T0 + DAY_MS,
        currency: USD,
      }),
    ).toThrow(FinanceDefinitionError);
    expect(() =>
      createFiscalPeriod({
        fiscalYear: 2025,
        periodType: StatementPeriodType.Annual,
        startMs: T0,
        endMs: T0,
        currency: USD,
      }),
    ).toThrow(FinanceDefinitionError);
  });

  it('创建公司资料并拒绝非法字段', () => {
    const profile = createCompanyProfile({
      instrument: aapl(),
      name: 'Apple Inc.',
      sector: 'Technology',
      industry: 'Consumer Electronics',
      country: 'US',
      currency: USD,
      sharesOutstanding: d('100'),
    });
    expect(profile.name).toBe('Apple Inc.');
    expect(Object.isFrozen(profile)).toBe(true);
    expect(() => createCompanyProfile({ instrument: aapl(), name: ' ', currency: USD })).toThrow(
      FinanceDefinitionError,
    );
    expect(() =>
      createCompanyProfile({
        instrument: aapl(),
        name: 'Apple',
        currency: USD,
        sharesOutstanding: Decimal.ZERO,
      }),
    ).toThrow(FinanceDefinitionError);
  });
});

describe('createFinancialStatements', () => {
  it('推导利润表、资产负债表与现金流量表合计', () => {
    const result = statements();
    expect(result.income.grossProfit.toString()).toBe('400');
    expect(result.income.operatingIncome.toString()).toBe('200');
    expect(result.income.pretaxIncome.toString()).toBe('180');
    expect(result.income.netIncome.toString()).toBe('144');
    expect(result.income.epsDiluted.toString()).toBe('1.44');
    expect(result.balance.currentAssets.toString()).toBe('450');
    expect(result.balance.totalAssets.toString()).toBe('1150');
    expect(result.balance.currentLiabilities.toString()).toBe('200');
    expect(result.balance.totalLiabilities.toString()).toBe('550');
    expect(result.balance.totalDebt.toString()).toBe('350');
    expect(result.balance.totalLiabilitiesAndEquity.toString()).toBe('1150');
    expect(result.cashFlow.freeCashFlow.toString()).toBe('130');
    expect(result.cashFlow.netChangeInCash.toString()).toBe('60');
    expect(Object.isFrozen(result)).toBe(true);
    expect(Object.isFrozen(result.income)).toBe(true);
    expect(Object.isFrozen(result.balance)).toBe(true);
    expect(Object.isFrozen(result.cashFlow)).toBe(true);
  });

  it('拒绝资产恒等式不成立、负收入、零股本与非法现金流', () => {
    expect(() =>
      statements(undefined, {
        balance: {
          cashAndEquivalents: d('100.01'),
        },
      }),
    ).toThrow(FinanceDefinitionError);
    expect(() => statements(undefined, { income: { revenue: d('-1') } })).toThrow(
      FinanceDefinitionError,
    );
    expect(() => statements(undefined, { income: { sharesOutstanding: Decimal.ZERO } })).toThrow(
      FinanceDefinitionError,
    );
    expect(() => statements(undefined, { cashFlow: { capitalExpenditure: d('-1') } })).toThrow(
      FinanceDefinitionError,
    );
    expect(() => statements(undefined, { balance: { shareholdersEquity: d('601') } })).toThrow(
      FinanceDefinitionError,
    );
  });
});

describe('computeFundamentalRatios', () => {
  it('计算核心比率', () => {
    const ratios = computeFundamentalRatios(statements());
    expect(ratios.currentRatio?.toString()).toBe('2.25');
    expect(ratios.quickRatio?.toString()).toBe('1.75');
    expect(ratios.cashRatio?.toString()).toBe('0.75');
    expect(ratios.workingCapital?.amount.toString()).toBe('250');
    expect(ratios.debtToEquity?.toString()).toBe('0.58333333');
    expect(ratios.debtToAssets?.toString()).toBe('0.30434783');
    expect(ratios.interestCoverage?.toString()).toBe('10');
    expect(ratios.grossMargin?.toString()).toBe('0.4');
    expect(ratios.operatingMargin?.toString()).toBe('0.2');
    expect(ratios.netMargin?.toString()).toBe('0.144');
    expect(ratios.returnOnAssets?.toString()).toBe('0.12521739');
    expect(ratios.returnOnEquity?.toString()).toBe('0.24');
    expect(ratios.returnOnInvestedCapital?.toString()).toBe('0.2');
    expect(ratios.returnOnCapitalEmployed?.toString()).toBe('0.21052632');
    expect(ratios.assetTurnover?.toString()).toBe('0.86956522');
    expect(ratios.inventoryTurnover?.toString()).toBe('6');
    expect(ratios.receivablesTurnover?.toString()).toBe('6.66666667');
    expect(ratios.daysInventoryOutstanding?.toString()).toBe('60.83333333');
    expect(ratios.daysSalesOutstanding?.toString()).toBe('54.75');
    expect(ratios.freeCashFlowMargin?.toString()).toBe('0.13');
    expect(ratios.earningsPerShare?.toString()).toBe('1.44');
    expect(ratios.bookValuePerShare?.toString()).toBe('6');
    expect(ratios.freeCashFlowPerShare?.toString()).toBe('1.3');
    expect(ratios.dividendPayoutRatio?.toString()).toBe('0.20833333');
    expect(ratios.effectiveTaxRate?.toString()).toBe('0.2');
  });

  it('提供上期时使用平均余额', () => {
    const ratios = computeFundamentalRatios(statements(), priorStatements());
    expect(ratios.returnOnAssets?.toString()).toBe('0.128');
    expect(ratios.returnOnEquity?.toString()).toBe('0.25714286');
    expect(ratios.assetTurnover?.toString()).toBe('0.88888889');
  });

  it('分母不成立时字段缺省', () => {
    const zero = statements(undefined, {
      income: {
        revenue: Decimal.ZERO,
        costOfRevenue: Decimal.ZERO,
        operatingExpenses: Decimal.ZERO,
        interestExpense: Decimal.ZERO,
        taxExpense: Decimal.ZERO,
        sharesOutstanding: d('100'),
      },
      balance: {
        cashAndEquivalents: Decimal.ZERO,
        shortTermInvestments: Decimal.ZERO,
        accountsReceivable: Decimal.ZERO,
        inventory: Decimal.ZERO,
        otherCurrentAssets: Decimal.ZERO,
        propertyPlantEquipment: Decimal.ZERO,
        goodwill: Decimal.ZERO,
        intangibleAssets: Decimal.ZERO,
        otherNonCurrentAssets: Decimal.ZERO,
        accountsPayable: Decimal.ZERO,
        shortTermDebt: Decimal.ZERO,
        otherCurrentLiabilities: Decimal.ZERO,
        longTermDebt: Decimal.ZERO,
        otherNonCurrentLiabilities: Decimal.ZERO,
        shareholdersEquity: Decimal.ZERO,
      },
      cashFlow: {
        operatingCashFlow: Decimal.ZERO,
        capitalExpenditure: Decimal.ZERO,
        investingCashFlow: Decimal.ZERO,
        financingCashFlow: Decimal.ZERO,
        depreciationAmortization: Decimal.ZERO,
        dividendsPaid: Decimal.ZERO,
      },
    });
    const ratios = computeFundamentalRatios(zero);
    expect(ratios.currentRatio).toBeUndefined();
    expect(ratios.netMargin).toBeUndefined();
    expect(ratios.returnOnEquity).toBeUndefined();
    expect(ratios.bookValuePerShare).toBeUndefined();
  });

  it('拒绝不可比期间', () => {
    const quarterly = statements({ periodType: StatementPeriodType.Quarterly });
    expect(() => computeFundamentalRatios(quarterly, priorStatements())).toThrow(
      FinanceDefinitionError,
    );
  });
});

describe('成长性与 DuPont', () => {
  it('计算增长率', () => {
    const growth = computeGrowthMetrics(statements(), priorStatements());
    expect(growth.revenueGrowth?.toString()).toBe('0.11111111');
    expect(growth.netIncomeGrowth?.toString()).toBe('0.2');
    expect(growth.epsGrowth?.toString()).toBe('0.2');
    expect(growth.freeCashFlowGrowth?.toString()).toBe('0.18181818');
    expect(growth.equityGrowth?.toString()).toBe('0.15384615');
  });

  it('上期分母非正时成长字段缺省', () => {
    const negativePrior = statements(
      { fiscalYear: 2024, startMs: T0 - 365 * DAY_MS, endMs: T0 },
      {
        income: {
          revenue: d('0'),
          costOfRevenue: d('0'),
          operatingExpenses: d('0'),
          taxExpense: d('0'),
        },
        balance: {
          shareholdersEquity: d('520'),
          cashAndEquivalents: d('100'),
          shortTermInvestments: d('50'),
          accountsReceivable: d('140'),
          inventory: d('110'),
          otherCurrentAssets: d('0'),
          propertyPlantEquipment: d('500'),
          goodwill: d('100'),
          intangibleAssets: d('50'),
          otherNonCurrentAssets: d('50'),
          accountsPayable: d('110'),
          shortTermDebt: d('50'),
          otherCurrentLiabilities: d('50'),
          longTermDebt: d('320'),
          otherNonCurrentLiabilities: d('50'),
        },
        cashFlow: { operatingCashFlow: d('-10'), capitalExpenditure: d('0') },
      },
    );
    const growth = computeGrowthMetrics(statements(), negativePrior);
    expect(growth.revenueGrowth).toBeUndefined();
    expect(growth.netIncomeGrowth).toBeUndefined();
    expect(growth.freeCashFlowGrowth).toBeUndefined();
  });

  it('DuPont 三因子与五因子', () => {
    const dupont = computeDuPont(statements(), priorStatements());
    expect(dupont.netMargin?.toString()).toBe('0.144');
    expect(dupont.assetTurnover?.toString()).toBe('0.88888889');
    expect(dupont.equityMultiplier?.toString()).toBe('2.00892857');
    expect(dupont.returnOnEquity?.toString()).toBe('0.25714286');
    expect(dupont.taxBurden?.toString()).toBe('0.8');
    expect(dupont.interestBurden?.toString()).toBe('0.9');
    expect(dupont.operatingMargin?.toString()).toBe('0.2');
    expect(dupont.fiveFactorRoe).toBeDefined();
  });

  it('五因子均为 8 位小数时不会触发 Decimal scale 溢出', () => {
    // 五个因子自然 scale 之和为 40，必须显式限制中间 scale，否则超过 Decimal 上限 30。
    const scaleHeavy = statements(
      {},
      {
        income: {
          revenue: d('9'),
          costOfRevenue: d('0'),
          operatingExpenses: d('2'),
          interestExpense: d('4'),
          taxExpense: d('2'),
          sharesOutstanding: d('1'),
        },
        balance: {
          cashAndEquivalents: d('11'),
          shortTermInvestments: d('0'),
          accountsReceivable: d('0'),
          inventory: d('0'),
          otherCurrentAssets: d('0'),
          propertyPlantEquipment: d('0'),
          goodwill: d('0'),
          intangibleAssets: d('0'),
          otherNonCurrentAssets: d('0'),
          accountsPayable: d('2'),
          shortTermDebt: d('0'),
          otherCurrentLiabilities: d('0'),
          longTermDebt: d('0'),
          otherNonCurrentLiabilities: d('0'),
          shareholdersEquity: d('9'),
          retainedEarnings: d('0'),
        },
      },
    );
    const dupont = computeDuPont(scaleHeavy);
    expect(dupont.taxBurden?.toString()).toBe('0.33333333');
    expect(dupont.interestBurden?.toString()).toBe('0.42857143');
    expect(dupont.operatingMargin?.toString()).toBe('0.77777778');
    expect(dupont.assetTurnover?.toString()).toBe('0.81818182');
    expect(dupont.equityMultiplier?.toString()).toBe('1.22222222');
    expect(dupont.fiveFactorRoe).toBeDefined();
    expect(dupont.fiveFactorRoe?.compare(d('0.11111111'))).toBe(0);
  });

  it('拒绝不可比期间', () => {
    expect(() =>
      computeGrowthMetrics(
        statements(),
        statements({ fiscalYear: 2025, periodType: StatementPeriodType.Quarterly }),
      ),
    ).toThrow(FinanceDefinitionError);
    expect(() =>
      computeDuPont(statements(), statements({ currency: createCurrencyForTest() })),
    ).toThrow(FinanceDefinitionError);
  });
});

describe('估值倍数与 Piotroski', () => {
  it('计算市值、EV 与相对倍数', () => {
    const multiples = computeValuationMultiples(statements(), {
      price: d('20'),
      growthRate: d('0.1'),
    });
    expect(multiples.marketCap?.amount.toString()).toBe('2000');
    expect(multiples.enterpriseValue?.amount.toString()).toBe('2200');
    expect(multiples.peRatio?.toString()).toBe('13.88888889');
    expect(multiples.pbRatio?.toString()).toBe('3.33333333');
    expect(multiples.psRatio?.toString()).toBe('2');
    expect(multiples.evToEbitda?.toString()).toBe('9.16666667');
    expect(multiples.evToSales?.toString()).toBe('2.2');
    expect(multiples.dividendYield?.toString()).toBe('0.015');
    expect(multiples.earningsYield?.toString()).toBe('0.072');
    expect(multiples.pegRatio?.toString()).toBe('1.38888889');
  });

  it('净利润/权益/收入非正时倍数缺省', () => {
    const loss = statements(undefined, {
      income: {
        revenue: d('0'),
        costOfRevenue: d('0'),
        operatingExpenses: d('0'),
        taxExpense: d('0'),
      },
      balance: {
        cashAndEquivalents: Decimal.ZERO,
        shortTermInvestments: Decimal.ZERO,
        accountsReceivable: Decimal.ZERO,
        inventory: Decimal.ZERO,
        otherCurrentAssets: Decimal.ZERO,
        propertyPlantEquipment: Decimal.ZERO,
        goodwill: Decimal.ZERO,
        intangibleAssets: Decimal.ZERO,
        otherNonCurrentAssets: Decimal.ZERO,
        accountsPayable: Decimal.ZERO,
        shortTermDebt: Decimal.ZERO,
        otherCurrentLiabilities: Decimal.ZERO,
        longTermDebt: Decimal.ZERO,
        otherNonCurrentLiabilities: Decimal.ZERO,
        shareholdersEquity: Decimal.ZERO,
      },
      cashFlow: {
        operatingCashFlow: Decimal.ZERO,
        capitalExpenditure: Decimal.ZERO,
        investingCashFlow: Decimal.ZERO,
        financingCashFlow: Decimal.ZERO,
        depreciationAmortization: Decimal.ZERO,
      },
    });
    const multiples = computeValuationMultiples(loss, { price: d('10') });
    expect(multiples.peRatio).toBeUndefined();
    expect(multiples.pbRatio).toBeUndefined();
    expect(multiples.psRatio).toBeUndefined();
    expect(multiples.evToEbitda).toBeUndefined();
  });

  it('Piotroski 9 分与准则明细', () => {
    const result = computePiotroskiFScore(statements(), priorStatements());
    expect(result.score).toBe(9);
    expect(result.criteria[PiotroskiCriterion.PositiveNetIncome]).toBe(true);
    expect(result.criteria[PiotroskiCriterion.DecreasingLeverage]).toBe(true);
    expect(Object.isFrozen(result.criteria)).toBe(true);
  });

  it('Piotroski 缺失比率时对应准则为 false', () => {
    const zero = statements(undefined, {
      income: {
        revenue: Decimal.ZERO,
        costOfRevenue: Decimal.ZERO,
        operatingExpenses: Decimal.ZERO,
        taxExpense: Decimal.ZERO,
      },
      balance: {
        cashAndEquivalents: Decimal.ZERO,
        shortTermInvestments: Decimal.ZERO,
        accountsReceivable: Decimal.ZERO,
        inventory: Decimal.ZERO,
        otherCurrentAssets: Decimal.ZERO,
        propertyPlantEquipment: Decimal.ZERO,
        goodwill: Decimal.ZERO,
        intangibleAssets: Decimal.ZERO,
        otherNonCurrentAssets: Decimal.ZERO,
        accountsPayable: Decimal.ZERO,
        shortTermDebt: Decimal.ZERO,
        otherCurrentLiabilities: Decimal.ZERO,
        longTermDebt: Decimal.ZERO,
        otherNonCurrentLiabilities: Decimal.ZERO,
        shareholdersEquity: Decimal.ZERO,
      },
      cashFlow: {
        operatingCashFlow: Decimal.ZERO,
        capitalExpenditure: Decimal.ZERO,
        investingCashFlow: Decimal.ZERO,
        financingCashFlow: Decimal.ZERO,
        depreciationAmortization: Decimal.ZERO,
      },
    });
    const result = computePiotroskiFScore(zero, priorStatements());
    expect(result.score).toBeLessThanOrEqual(9);
    expect(result.criteria[PiotroskiCriterion.ImprovingRoa]).toBe(false);
  });

  it('拒绝非法估值输入', () => {
    expect(() => computeValuationMultiples(statements(), { price: Decimal.ZERO })).toThrow(
      FinanceDefinitionError,
    );
    expect(() =>
      computeValuationMultiples(statements(), { price: d('10'), sharesOutstanding: Decimal.ZERO }),
    ).toThrow(FinanceDefinitionError);
  });
});

/** 便于测试不可比货币。 */
function createCurrencyForTest(): Currency {
  return createCurrency('EUR', 2);
}
