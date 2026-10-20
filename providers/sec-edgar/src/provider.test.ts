/**
 * @file SEC EDGAR 提供方测试。
 * @description 用固定 tickers/companyfacts JSON 覆盖参考数据、公司资料与报表映射。
 * @module sec-edgar/provider.test
 */

import { describe, expect, it } from 'vitest';

import {
  ProviderDataError,
  StatementPeriodType,
  createCurrency,
  createInstrument,
  type FetchLike,
  type Instrument,
} from '@kurumi/finance';
import { Decimal } from '@kurumi/math';

import { createSecEdgarFundamentalProvider, createSecEdgarReferenceProvider } from './index.js';

const USD = createCurrency('USD', 2);
const aapl = createInstrument({
  assetClass: 'equity',
  venue: 'sec',
  symbol: 'AAPL',
  baseAsset: 'AAPL',
  quoteCurrency: USD,
  tickSize: Decimal.parse('0.01'),
  lotSize: Decimal.ONE,
}) as Instrument;
const USER_AGENT = 'kurumi-test/0.1 test@example.com';

function fact(value: number, end: string, start?: string): Record<string, unknown> {
  return {
    ...(start === undefined ? {} : { start }),
    end,
    val: value,
    fy: 2025,
    fp: 'FY',
    form: '10-K',
    filed: '2026-02-01',
  };
}

function unit(entries: readonly Record<string, unknown>[]) {
  return { units: { USD: entries } };
}

const companyFacts = {
  cik: 320193,
  entityName: 'Apple Inc.',
  facts: {
    'us-gaap': {
      Revenues: unit([fact(1000, '2025-12-31', '2025-01-01')]),
      CostOfRevenue: unit([fact(600, '2025-12-31', '2025-01-01')]),
      OperatingIncomeLoss: unit([fact(200, '2025-12-31', '2025-01-01')]),
      InterestExpense: unit([fact(20, '2025-12-31', '2025-01-01')]),
      IncomeTaxExpenseBenefit: unit([fact(36, '2025-12-31', '2025-01-01')]),
      NetIncomeLoss: unit([fact(144, '2025-12-31', '2025-01-01')]),
      WeightedAverageNumberOfDilutedSharesOutstanding: {
        units: { shares: [fact(100, '2025-12-31', '2025-01-01')] },
      },
      AssetsCurrent: unit([fact(450, '2025-12-31')]),
      Assets: unit([fact(1150, '2025-12-31')]),
      LiabilitiesCurrent: unit([fact(200, '2025-12-31')]),
      Liabilities: unit([fact(550, '2025-12-31')]),
      StockholdersEquity: unit([fact(600, '2025-12-31')]),
      CashAndCashEquivalentsAtCarryingValue: unit([fact(100, '2025-12-31')]),
      ShortTermInvestments: unit([fact(50, '2025-12-31')]),
      AccountsReceivableNetCurrent: unit([fact(150, '2025-12-31')]),
      InventoryNet: unit([fact(100, '2025-12-31')]),
      PropertyPlantAndEquipmentNet: unit([fact(500, '2025-12-31')]),
      Goodwill: unit([fact(100, '2025-12-31')]),
      IntangibleAssetsNetExcludingGoodwill: unit([fact(50, '2025-12-31')]),
      AccountsPayableCurrent: unit([fact(100, '2025-12-31')]),
      ShortTermBorrowings: unit([fact(50, '2025-12-31')]),
      LongTermDebtNoncurrent: unit([fact(300, '2025-12-31')]),
      RetainedEarningsAccumulatedDeficit: unit([fact(200, '2025-12-31')]),
      NetCashProvidedByUsedInOperatingActivities: unit([fact(180, '2025-12-31', '2025-01-01')]),
      PaymentsToAcquirePropertyPlantAndEquipment: unit([fact(50, '2025-12-31', '2025-01-01')]),
      NetCashProvidedByUsedInInvestingActivities: unit([fact(-100, '2025-12-31', '2025-01-01')]),
      NetCashProvidedByUsedInFinancingActivities: unit([fact(-20, '2025-12-31', '2025-01-01')]),
      DepreciationDepletionAndAmortization: unit([fact(40, '2025-12-31', '2025-01-01')]),
      PaymentsOfDividends: unit([fact(30, '2025-12-31', '2025-01-01')]),
      PaymentsForRepurchaseOfCommonStock: unit([fact(10, '2025-12-31', '2025-01-01')]),
    },
  },
};

const tickers = {
  '0': { cik_str: 320193, ticker: 'AAPL', title: 'Apple Inc.' },
};

function fetchFor(routes: Readonly<Record<string, unknown>>): FetchLike {
  return async (input) => {
    const path = new URL(input).pathname;
    const payload = routes[path];
    if (payload === undefined) return new Response('{}', { status: 404 });
    return new Response(JSON.stringify(payload), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    });
  };
}

const routes = {
  '/files/company_tickers.json': tickers,
  '/api/xbrl/companyfacts/CIK0000320193.json': companyFacts,
};

describe('SEC EDGAR reference data', () => {
  it('映射股票标的与货币', async () => {
    const provider = createSecEdgarReferenceProvider({
      fetchImpl: fetchFor(routes),
      userAgent: USER_AGENT,
    });
    const instruments = await provider.listInstruments();
    expect(instruments).toHaveLength(1);
    expect(instruments[0]?.symbol).toBe('AAPL');
    expect(instruments[0]?.venue).toBe('sec');
    expect((await provider.getInstrument('sec:AAPL'))?.symbol).toBe('AAPL');
    expect(await provider.getInstrument('sec:NOPE')).toBeUndefined();
    expect((await provider.listCurrencies()).map((item) => item.code)).toEqual(['USD']);
  });

  it('User-Agent 必填且包含联系方式', () => {
    expect(() => createSecEdgarReferenceProvider({ userAgent: '' })).toThrow(ProviderDataError);
    expect(() => createSecEdgarReferenceProvider({ userAgent: 'kurumi' })).toThrow(
      ProviderDataError,
    );
  });
});

describe('SEC EDGAR fundamentals', () => {
  it('映射公司资料与年度报表', async () => {
    const provider = createSecEdgarFundamentalProvider({
      fetchImpl: fetchFor(routes),
      userAgent: USER_AGENT,
    });
    const profile = await provider.getCompanyProfile?.(aapl);
    expect(profile?.name).toBe('Apple Inc.');
    expect(profile?.country).toBe('US');

    const statements = await provider.getFinancialStatements(aapl, {
      periods: 1,
      periodType: StatementPeriodType.Annual,
    });
    expect(statements).toHaveLength(1);
    const income = statements[0]?.income;
    expect(income?.revenue.toString()).toBe('1000');
    expect(income?.grossProfit.toString()).toBe('400');
    expect(income?.operatingIncome.toString()).toBe('200');
    expect(income?.netIncome.toString()).toBe('144');
    expect(income?.epsDiluted.toString()).toBe('1.44');
    expect(statements[0]?.balance.totalAssets.toString()).toBe('1150');
    expect(statements[0]?.balance.totalLiabilities.toString()).toBe('550');
    expect(statements[0]?.cashFlow.freeCashFlow.toString()).toBe('130');
  });

  it('季度报表选择单季度 fact，而不是年初至今 fact', async () => {
    const quarterly = JSON.parse(JSON.stringify(companyFacts)) as typeof companyFacts;
    const usGaap = quarterly.facts['us-gaap'] as Record<string, unknown>;
    for (const value of Object.values(usGaap)) {
      if (typeof value !== 'object' || value === null) continue;
      const units = (value as { units?: Record<string, unknown> }).units;
      if (units === undefined) continue;
      for (const entries of Object.values(units)) {
        if (!Array.isArray(entries)) continue;
        for (const entry of entries) {
          if (typeof entry !== 'object' || entry === null) continue;
          const record = entry as Record<string, unknown>;
          record['form'] = '10-Q';
          record['end'] = '2025-03-31';
          if (record['start'] !== undefined) record['start'] = '2025-01-01';
          record['filed'] = '2025-05-01';
        }
      }
    }
    const netIncome = (usGaap['NetIncomeLoss'] as { units: { USD: Record<string, unknown>[] } })
      .units.USD;
    netIncome.push({
      start: '2024-10-01',
      end: '2025-03-31',
      val: 288,
      fy: 2025,
      fp: 'Q2',
      form: '10-Q',
      filed: '2025-05-02',
    });

    const provider = createSecEdgarFundamentalProvider({
      fetchImpl: fetchFor({
        ...routes,
        '/api/xbrl/companyfacts/CIK0000320193.json': quarterly,
      }),
      userAgent: USER_AGENT,
    });
    const statements = await provider.getFinancialStatements(aapl, {
      periods: 1,
      periodType: StatementPeriodType.Quarterly,
    });
    expect(statements[0]?.period.startMs).toBe(Date.UTC(2025, 0, 1));
    expect(statements[0]?.income.revenue.toString()).toBe('1000');
  });

  it('股本缺失时显式失败，不回退为 1', async () => {
    const missingShares = JSON.parse(JSON.stringify(companyFacts)) as typeof companyFacts;
    const usGaap = missingShares.facts['us-gaap'] as Record<string, unknown>;
    delete usGaap['WeightedAverageNumberOfDilutedSharesOutstanding'];
    const provider = createSecEdgarFundamentalProvider({
      fetchImpl: fetchFor({
        ...routes,
        '/api/xbrl/companyfacts/CIK0000320193.json': missingShares,
      }),
      userAgent: USER_AGENT,
    });
    await expect(provider.getFinancialStatements(aapl, { periods: 1 })).rejects.toBeInstanceOf(
      ProviderDataError,
    );
  });

  it('过滤参考数据并处理未知公司', async () => {
    const provider = createSecEdgarReferenceProvider({
      fetchImpl: fetchFor(routes),
      userAgent: USER_AGENT,
    });
    expect(await provider.listInstruments({ assetClass: 'crypto' })).toEqual([]);
    expect(await provider.listInstruments({ venue: 'nasdaq' })).toEqual([]);
    await expect(provider.listInstruments({ venue: 'sec' })).resolves.toHaveLength(1);
    const unknown = createInstrument({
      assetClass: 'equity',
      venue: 'sec',
      symbol: 'NOPE',
      baseAsset: 'NOPE',
      quoteCurrency: USD,
      tickSize: Decimal.parse('0.01'),
      lotSize: Decimal.ONE,
    });
    const fundamentals = createSecEdgarFundamentalProvider({
      fetchImpl: fetchFor(routes),
      userAgent: USER_AGENT,
    });
    await expect(fundamentals.getCompanyProfile?.(unknown)).resolves.toBeUndefined();
    await expect(fundamentals.getFinancialStatements(unknown)).rejects.toBeInstanceOf(
      ProviderDataError,
    );
  });

  it('拒绝非法 periods、缺失净利润与缺失收入', async () => {
    const provider = createSecEdgarFundamentalProvider({
      fetchImpl: fetchFor(routes),
      userAgent: USER_AGENT,
    });
    await expect(provider.getFinancialStatements(aapl, { periods: 0 })).rejects.toBeInstanceOf(
      ProviderDataError,
    );

    const noNetIncome = JSON.parse(JSON.stringify(companyFacts)) as typeof companyFacts;
    delete (noNetIncome.facts['us-gaap'] as Record<string, unknown>)['NetIncomeLoss'];
    const noNet = createSecEdgarFundamentalProvider({
      fetchImpl: fetchFor({
        '/files/company_tickers.json': tickers,
        '/api/xbrl/companyfacts/CIK0000320193.json': noNetIncome,
      }),
      userAgent: USER_AGENT,
    });
    await expect(noNet.getFinancialStatements(aapl)).rejects.toBeInstanceOf(ProviderDataError);

    const noRevenue = JSON.parse(JSON.stringify(companyFacts)) as typeof companyFacts;
    delete (noRevenue.facts['us-gaap'] as Record<string, unknown>)['Revenues'];
    const noRev = createSecEdgarFundamentalProvider({
      fetchImpl: fetchFor({
        '/files/company_tickers.json': tickers,
        '/api/xbrl/companyfacts/CIK0000320193.json': noRevenue,
      }),
      userAgent: USER_AGENT,
    });
    await expect(noRev.getFinancialStatements(aapl)).rejects.toBeInstanceOf(ProviderDataError);
  });

  it('处理缺失 start/股本、非法数值与非法日期', async () => {
    const noStart = JSON.parse(JSON.stringify(companyFacts)) as typeof companyFacts;
    const noStartFacts = noStart.facts['us-gaap'] as Record<string, unknown>;
    const netIncome = noStartFacts['NetIncomeLoss'] as {
      units: { USD: Record<string, unknown>[] };
    };
    if (netIncome.units.USD[0] !== undefined) delete netIncome.units.USD[0]['start'];
    const provider = createSecEdgarFundamentalProvider({
      fetchImpl: fetchFor({
        '/files/company_tickers.json': tickers,
        '/api/xbrl/companyfacts/CIK0000320193.json': noStart,
      }),
      userAgent: USER_AGENT,
    });
    const statements = await provider.getFinancialStatements(aapl, { periods: 1 });
    expect(statements[0]?.period.startMs).toBe(Date.UTC(2025, 0, 1));

    const invalidValue = JSON.parse(JSON.stringify(companyFacts)) as typeof companyFacts;
    const invalidFacts = invalidValue.facts['us-gaap'] as Record<string, unknown>;
    const invalidNet = invalidFacts['NetIncomeLoss'] as {
      units: { USD: Record<string, unknown>[] };
    };
    if (invalidNet.units.USD[0] !== undefined) invalidNet.units.USD[0]['val'] = 'abc';
    const invalidProvider = createSecEdgarFundamentalProvider({
      fetchImpl: fetchFor({
        '/files/company_tickers.json': tickers,
        '/api/xbrl/companyfacts/CIK0000320193.json': invalidValue,
      }),
      userAgent: USER_AGENT,
    });
    await expect(invalidProvider.getFinancialStatements(aapl)).rejects.toBeInstanceOf(
      ProviderDataError,
    );

    const invalidDate = JSON.parse(JSON.stringify(companyFacts)) as typeof companyFacts;
    const dateFacts = invalidDate.facts['us-gaap'] as Record<string, unknown>;
    const dateNet = dateFacts['NetIncomeLoss'] as { units: { USD: Record<string, unknown>[] } };
    if (dateNet.units.USD[0] !== undefined) dateNet.units.USD[0]['start'] = 'bad-date';
    const dateProvider = createSecEdgarFundamentalProvider({
      fetchImpl: fetchFor({
        '/files/company_tickers.json': tickers,
        '/api/xbrl/companyfacts/CIK0000320193.json': invalidDate,
      }),
      userAgent: USER_AGENT,
    });
    await expect(dateProvider.getFinancialStatements(aapl)).rejects.toBeInstanceOf(
      ProviderDataError,
    );
  });

  it('负税费走 maxZero 分支', async () => {
    const negativeTax = JSON.parse(JSON.stringify(companyFacts)) as typeof companyFacts;
    const facts = negativeTax.facts['us-gaap'] as Record<string, unknown>;
    const tax = facts['IncomeTaxExpenseBenefit'] as { units: { USD: Record<string, unknown>[] } };
    if (tax.units.USD[0] !== undefined) tax.units.USD[0]['val'] = -10;
    const provider = createSecEdgarFundamentalProvider({
      fetchImpl: fetchFor({
        '/files/company_tickers.json': tickers,
        '/api/xbrl/companyfacts/CIK0000320193.json': negativeTax,
      }),
      userAgent: USER_AGENT,
    });
    const statements = await provider.getFinancialStatements(aapl, { periods: 1 });
    expect(statements[0]?.income.taxExpense.isZero()).toBe(true);
    expect(statements[0]?.income.netIncome.toString()).toBe('144');
  });

  it('资产负债表 residuals 为负时失败', async () => {
    const broken = JSON.parse(JSON.stringify(companyFacts)) as typeof companyFacts;
    const facts = broken.facts['us-gaap'] as Record<string, unknown>;
    delete facts['CashAndCashEquivalentsAtCarryingValue'];
    facts['AssetsCurrent'] = unit([fact(10, '2025-12-31')]);
    const provider = createSecEdgarFundamentalProvider({
      fetchImpl: fetchFor({
        '/files/company_tickers.json': tickers,
        '/api/xbrl/companyfacts/CIK0000320193.json': broken,
      }),
      userAgent: USER_AGENT,
    });
    await expect(provider.getFinancialStatements(aapl)).rejects.toBeInstanceOf(ProviderDataError);
  });

  it('缺少关键资产负债表字段时失败', async () => {
    const broken = JSON.parse(JSON.stringify(companyFacts)) as typeof companyFacts;
    delete (broken.facts['us-gaap'] as Record<string, unknown>)['Assets'];
    const provider = createSecEdgarFundamentalProvider({
      fetchImpl: fetchFor({
        '/files/company_tickers.json': tickers,
        '/api/xbrl/companyfacts/CIK0000320193.json': broken,
      }),
      userAgent: USER_AGENT,
    });
    await expect(provider.getFinancialStatements(aapl)).rejects.toBeInstanceOf(ProviderDataError);
  });

  it('TTM 请求显式失败', async () => {
    const provider = createSecEdgarFundamentalProvider({
      fetchImpl: fetchFor(routes),
      userAgent: USER_AGENT,
    });
    await expect(
      provider.getFinancialStatements(aapl, { periodType: StatementPeriodType.Ttm }),
    ).rejects.toBeInstanceOf(ProviderDataError);
  });
});
