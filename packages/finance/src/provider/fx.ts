/**
 * @file 只读 FX 汇率提供方契约。
 * @description 查询单条或全部汇率；不包含下单、兑换或资金划转。
 * @module finance/provider/fx
 */

import type { Bar } from '../market-data.js';
import type { GetBarsOptions } from './market-data.js';
import type { FxRate } from '../fx.js';
import type { FxInstrument } from '../instrument.js';
import type { Currency } from '../money.js';

/** 只读 FX 汇率提供方。 */
export interface FxRateProvider {
  /** 提供方 id。 */
  readonly id: string;
  /**
   * 查询单条汇率。
   *
   * @param base - 源货币
   * @param quote - 目标货币
   * @param signal - 取消信号
   */
  getRate(base: Currency, quote: Currency, signal?: AbortSignal): Promise<FxRate | undefined>;
  /**
   * 列出全部可用汇率。
   *
   * @param signal - 取消信号
   */
  listRates(signal?: AbortSignal): Promise<readonly FxRate[]>;
}

/** 只读 FX K 线提供方。 */
export interface FxBarProvider {
  /** 提供方 id。 */
  readonly id: string;
  /**
   * 获取 FX K 线。
   *
   * @param instrument - FX 标的
   * @param options - 周期与时间范围
   * @param signal - 取消信号
   */
  getBars(
    instrument: FxInstrument,
    options: GetBarsOptions,
    signal?: AbortSignal,
  ): Promise<readonly Bar[]>;
}
