/**
 * @file 只读期权链提供方契约。
 * @description 提供到期日列表与按到期日筛选的期权合约报价；不包含下单/行权/结算。
 * @module finance/provider/options
 */

import type { Decimal } from '@kurumi/math';

import type { Instrument } from '../instrument.js';
import type { OptionType } from '../instrument.js';

/** 单张期权合约的只读报价。 */
export interface OptionContractQuote {
  /** 合约代码（数据源定义）。 */
  readonly contractId: string;
  /** call / put。 */
  readonly optionType: OptionType;
  /** 行权价。 */
  readonly strike: Decimal;
  /** 到期时间（epoch 毫秒，UTC 收盘）。 */
  readonly expirationMs: number;
  /** 买价。 */
  readonly bid?: Decimal;
  /** 卖价。 */
  readonly ask?: Decimal;
  /** 最新价。 */
  readonly last?: Decimal;
  /** 成交量。 */
  readonly volume?: Decimal;
  /** 未平仓量。 */
  readonly openInterest?: Decimal;
  /** 隐含波动率（小数，0.2 表示 20%）。 */
  readonly impliedVolatility?: Decimal;
  /** 数据源时间（epoch 毫秒）。 */
  readonly timestampMs: number;
}

/** 单次到期日的期权链。 */
export interface OptionsChain {
  /** 标的。 */
  readonly underlying: Instrument;
  /** 到期时间（epoch 毫秒）。 */
  readonly expirationMs: number;
  /** 合约报价。 */
  readonly contracts: readonly OptionContractQuote[];
  /** 数据源时间（epoch 毫秒）。 */
  readonly timestampMs: number;
}

/** 只读期权链提供方。 */
export interface OptionsDataProvider {
  /** 提供方 id。 */
  readonly id: string;
  /**
   * 列出可用到期日。
   *
   * @param underlying - 标的
   * @param signal - 取消信号
   */
  getExpirations(underlying: Instrument, signal?: AbortSignal): Promise<readonly number[]>;
  /**
   * 获取指定到期日的期权链。
   *
   * @param underlying - 标的
   * @param expirationMs - 到期时间（必须来自 getExpirations）
   * @param signal - 取消信号
   */
  getChain(
    underlying: Instrument,
    expirationMs: number,
    signal?: AbortSignal,
  ): Promise<OptionsChain>;
}
