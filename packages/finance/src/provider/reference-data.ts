/**
 * @file 只读参考数据提供方契约。
 * @description 查询标的、货币与交易所元数据；不包含任何账户或交易写操作。
 * @module finance/provider/reference-data
 */

import type { AssetClass, Instrument } from '../instrument.js';
import type { Currency } from '../money.js';

/** 标的查询选项。 */
export interface ListInstrumentsOptions {
  /** 限定交易场所。 */
  readonly venue?: string;
  /** 限定资产类别。 */
  readonly assetClass?: AssetClass;
  /** 取消信号。 */
  readonly signal?: AbortSignal;
}

/** 只读参考数据提供方。 */
export interface ReferenceDataProvider {
  /** 提供方 id。 */
  readonly id: string;
  /**
   * 列出标的。
   *
   * @param options - 场所、资产类别与取消信号
   */
  listInstruments(options?: ListInstrumentsOptions): Promise<readonly Instrument[]>;
  /**
   * 按 key 查询标的。
   *
   * @param key - `${venue}:${symbol}` 或提供方定义的稳定 id
   * @param signal - 取消信号
   */
  getInstrument(key: string, signal?: AbortSignal): Promise<Instrument | undefined>;
  /**
   * 列出支持的货币。
   *
   * @param signal - 取消信号
   */
  listCurrencies(signal?: AbortSignal): Promise<readonly Currency[]>;
}
