/**
 * @file 只读基本面数据提供方契约。
 * @description 查询公司资料与财务报表；不包含交易执行或账户操作。
 * @module finance/provider/fundamentals
 */

import type { CompanyProfile, FinancialStatements, StatementPeriodType } from '../fundamentals.js';
import type { Instrument } from '../instrument.js';

/** 财务报表查询选项。 */
export interface GetFinancialStatementsOptions {
  /** 期数上限。 */
  readonly periods?: number;
  /** 期间类型；缺省由提供方决定。 */
  readonly periodType?: StatementPeriodType;
  /** 取消信号。 */
  readonly signal?: AbortSignal;
}

/** 只读基本面数据提供方。 */
export interface FundamentalDataProvider {
  /** 提供方 id。 */
  readonly id: string;
  /**
   * 获取公司资料（可选能力）。
   *
   * @param instrument - 股票标的
   * @param signal - 取消信号
   */
  getCompanyProfile?(
    instrument: Instrument,
    signal?: AbortSignal,
  ): Promise<CompanyProfile | undefined>;
  /**
   * 获取财务报表，按报告期从新到旧排列。
   *
   * @param instrument - 股票标的
   * @param options - 期数、期间类型与取消信号
   */
  getFinancialStatements(
    instrument: Instrument,
    options?: GetFinancialStatementsOptions,
  ): Promise<readonly FinancialStatements[]>;
}
