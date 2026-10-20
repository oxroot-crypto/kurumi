/**
 * @file 只读债券数据提供方契约。
 * @description 查询债券静态规格；不包含报价、成交或交易执行。
 * @module finance/provider/fixed-income
 */

import type { Decimal } from '@kurumi/math';

import type { BondSpec } from '../fixed-income.js';
import type { Currency } from '../money.js';

/** 债券查询选项。 */
export interface ListBondsOptions {
  /** 限定计价货币。 */
  readonly currency?: Currency;
  /** 取消信号。 */
  readonly signal?: AbortSignal;
}

/** 只读债券数据提供方。 */
export interface BondDataProvider {
  /** 提供方 id。 */
  readonly id: string;
  /**
   * 按 id 查询债券。
   *
   * @param bondId - 债券 id
   * @param signal - 取消信号
   */
  getBond(bondId: string, signal?: AbortSignal): Promise<BondSpec | undefined>;
  /**
   * 列出债券。
   *
   * @param options - 货币与取消信号
   */
  listBonds(options?: ListBondsOptions): Promise<readonly BondSpec[]>;
}

/** 收益率曲线上的一个期限点。 */
export interface YieldCurvePoint {
  /** 期限（年）。 */
  readonly tenorYears: number;
  /** 年化收益率（小数，0.04 表示 4%）。 */
  readonly yieldRate: Decimal;
}

/** 单日收益率曲线。 */
export interface YieldCurve {
  /** 曲线日期（YYYY-MM-DD）。 */
  readonly date: string;
  /** 货币。 */
  readonly currency: Currency;
  /** 期限点，按 tenor 升序。 */
  readonly points: readonly YieldCurvePoint[];
}

/** 只读收益率曲线提供方。 */
export interface YieldCurveProvider {
  /** 提供方 id。 */
  readonly id: string;
  /**
   * 获取最新可用曲线。
   *
   * @param currency - 货币
   * @param signal - 取消信号
   */
  getLatest(currency: Currency, signal?: AbortSignal): Promise<YieldCurve>;
  /**
   * 获取指定日期曲线。
   *
   * @param date - YYYY-MM-DD
   * @param currency - 货币
   * @param signal - 取消信号
   */
  getCurve(date: string, currency: Currency, signal?: AbortSignal): Promise<YieldCurve>;
}
