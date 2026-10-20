/**
 * @file Nasdaq 只读股票期权链提供方。
 * @description 使用 api.nasdaq.com 公开 option-chain 接口；返回按到期日筛选的 call/put 合约报价。
 * @module nasdaq/options-chain
 */

import {
  OptionType,
  ProviderDataError,
  type Instrument,
  type OptionContractQuote,
  type OptionsChain,
  type OptionsDataProvider,
} from '@kurumi/finance';
import { Decimal } from '@kurumi/math';

import {
  getNasdaqJson,
  parseDecimalField,
  parseLongDate,
  requireArray,
  requireRecord,
  resolveNasdaqOptions,
  type NasdaqProviderOptions,
  type ResolvedNasdaqOptions,
} from './support.js';

/**
 * 创建 Nasdaq 期权链提供方。
 *
 * @param options - base URL、fetch 与资产类别（股票/ETF/指数）
 * @returns 只读 `OptionsDataProvider`
 *
 * @example
 * ```ts
 * const provider = createNasdaqOptionsProvider({ assetClass: 'etf' });
 * const expirations = await provider.getExpirations(spy);
 * const chain = await provider.getChain(spy, expirations[0]!);
 * ```
 */
export function createNasdaqOptionsProvider(
  options: NasdaqProviderOptions = {},
): OptionsDataProvider {
  const settings = resolveNasdaqOptions(options);
  return Object.freeze({
    id: settings.id,
    getExpirations: (underlying: Instrument, signal?: AbortSignal) =>
      getExpirations(settings, underlying, signal),
    getChain: (underlying: Instrument, expirationMs: number, signal?: AbortSignal) =>
      getChain(settings, underlying, expirationMs, signal),
  });
}

/** 获取到期日列表。 */
async function getExpirations(
  settings: ResolvedNasdaqOptions,
  underlying: Instrument,
  signal?: AbortSignal,
): Promise<readonly number[]> {
  const rows = await fetchRows(settings, underlying, signal);
  const expirations = new Set<number>();
  for (const row of rows) {
    const group = row['expirygroup'];
    if (typeof group === 'string' && group.trim() !== '') {
      expirations.add(parseLongDate(group, settings.id));
    }
  }
  return Object.freeze([...expirations].sort((left, right) => left - right));
}

/** 获取指定到期日期权链。 */
async function getChain(
  settings: ResolvedNasdaqOptions,
  underlying: Instrument,
  expirationMs: number,
  signal?: AbortSignal,
): Promise<OptionsChain> {
  if (!Number.isSafeInteger(expirationMs) || expirationMs <= 0) {
    throw new ProviderDataError(settings.id, 'expirationMs 必须是正安全整数');
  }
  const rows = await fetchRows(settings, underlying, signal);
  const timestampMs = settings.nowMs();
  const contracts: OptionContractQuote[] = [];
  let currentExpiration: number | undefined;
  let matched = false;

  for (let index = 0; index < rows.length; index += 1) {
    const row = rows[index];
    if (row === undefined) continue;
    const group = row['expirygroup'];
    if (typeof group === 'string' && group.trim() !== '') {
      currentExpiration = parseLongDate(group, settings.id);
    }
    if (currentExpiration === undefined || currentExpiration !== expirationMs) continue;
    matched = true;

    const strike = parseDecimalField(row['strike'], `rows[${index}].strike`, settings.id);
    if (strike === undefined) continue;
    for (const optionType of [OptionType.Call, OptionType.Put] as const) {
      const contract = buildContract(
        settings,
        underlying,
        row,
        index,
        optionType,
        strike,
        expirationMs,
        timestampMs,
      );
      if (contract !== undefined) contracts.push(contract);
    }
  }

  if (!matched) {
    throw new ProviderDataError(
      settings.id,
      `未找到到期日 ${new Date(expirationMs).toISOString().slice(0, 10)}`,
    );
  }

  return Object.freeze({
    underlying,
    expirationMs,
    contracts: Object.freeze(contracts),
    timestampMs,
  });
}

/** 构建单张合约；所有报价字段缺失时返回 undefined。 */
function buildContract(
  settings: ResolvedNasdaqOptions,
  underlying: Instrument,
  row: Record<string, unknown>,
  index: number,
  optionType: OptionType,
  strike: Decimal,
  expirationMs: number,
  timestampMs: number,
): OptionContractQuote | undefined {
  const prefix = optionType === OptionType.Call ? 'c' : 'p';
  const last = parseDecimalField(
    row[`${prefix}_Last`],
    `rows[${index}].${prefix}_Last`,
    settings.id,
  );
  const bid = parseDecimalField(row[`${prefix}_Bid`], `rows[${index}].${prefix}_Bid`, settings.id);
  const ask = parseDecimalField(row[`${prefix}_Ask`], `rows[${index}].${prefix}_Ask`, settings.id);
  const volume = parseDecimalField(
    row[`${prefix}_Volume`],
    `rows[${index}].${prefix}_Volume`,
    settings.id,
  );
  const openInterest = parseDecimalField(
    row[`${prefix}_Openinterest`],
    `rows[${index}].${prefix}_Openinterest`,
    settings.id,
  );
  if (
    last === undefined &&
    bid === undefined &&
    ask === undefined &&
    volume === undefined &&
    openInterest === undefined
  ) {
    return undefined;
  }

  return Object.freeze({
    contractId: `${underlying.venue}:${underlying.symbol}:${optionType}:${strike.toString()}:${expirationMs}`,
    optionType,
    strike,
    expirationMs,
    ...(bid === undefined ? {} : { bid }),
    ...(ask === undefined ? {} : { ask }),
    ...(last === undefined ? {} : { last }),
    ...(volume === undefined ? {} : { volume }),
    ...(openInterest === undefined ? {} : { openInterest }),
    timestampMs,
  });
}

/** 拉取并校验 option-chain 行。 */
async function fetchRows(
  settings: ResolvedNasdaqOptions,
  underlying: Instrument,
  signal?: AbortSignal,
): Promise<readonly Record<string, unknown>[]> {
  const payload = await getNasdaqJson(
    settings,
    `/api/quote/${encodeURIComponent(underlying.symbol)}/option-chain`,
    { assetclass: settings.assetClass, limit: String(settings.maxRows) },
    signal,
  );
  const root = requireRecord(payload, '$', settings.id);
  const data = requireRecord(root['data'], '$.data', settings.id);
  const table = requireRecord(data['table'], '$.data.table', settings.id);
  const rows = requireArray(table['rows'], '$.data.table.rows', settings.id);
  return Object.freeze(
    rows.map((row, index) => requireRecord(row, `$.data.table.rows[${index}]`, settings.id)),
  );
}
