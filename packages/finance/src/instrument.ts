/**
 * @file 金融标的与合约规格：股票、加密货币、FX、期货、永续与期权。
 * @description 定义判别联合、标的注册表、tick/lot 校验、价格/数量对齐与名义价值计算。
 * @module finance/instrument
 */

import { isPlainObject } from '@kurumi/core';

import {
  Decimal,
  isDecimal,
  isDecimalMultipleOf,
  roundDecimalToStep,
  type DecimalRounding,
} from '@kurumi/math';
import {
  CurrencyMismatchError,
  FinanceDefinitionError,
  InstrumentMismatchError,
} from './errors.js';
import {
  createMoney,
  currencyEquals,
  currencyLabel,
  isCurrency,
  isMoney,
  type Currency,
  type Money,
} from './money.js';
import {
  assertBoolean,
  assertNonEmptyString,
  assertOneOf,
  assertPattern,
  assertPositiveDecimal,
  assertPositiveSafeInteger,
} from './validation.js';

/** 资产类别。 */
export const AssetClass = {
  /** 股票/ETF。 */
  Equity: 'equity',
  /** 加密货币现货或币对。 */
  Crypto: 'crypto',
  /** 外汇货币对。 */
  Fx: 'fx',
  /** 到期交割/现金结算期货。 */
  Future: 'future',
  /** 永续合约。 */
  Perpetual: 'perpetual',
  /** 期权。 */
  Option: 'option',
  /** 指数。 */
  Index: 'index',
} as const;

/** 资产类别联合类型。 */
export type AssetClass = (typeof AssetClass)[keyof typeof AssetClass];

/** 全部资产类别。 */
const ASSET_CLASSES: readonly AssetClass[] = Object.freeze(Object.values(AssetClass));

/** 结算方式。 */
export const SettlementType = {
  /** 现金结算。 */
  Cash: 'cash',
  /** 实物交割。 */
  Physical: 'physical',
} as const;

/** 结算方式联合类型。 */
export type SettlementType = (typeof SettlementType)[keyof typeof SettlementType];

/** 结算方式集合。 */
const SETTLEMENT_TYPES: readonly SettlementType[] = Object.freeze(Object.values(SettlementType));

/** 期权方向。 */
export const OptionType = {
  /** 看涨期权。 */
  Call: 'call',
  /** 看跌期权。 */
  Put: 'put',
} as const;

/** 期权方向联合类型。 */
export type OptionType = (typeof OptionType)[keyof typeof OptionType];

/** 期权方向集合。 */
const OPTION_TYPES: readonly OptionType[] = Object.freeze(Object.values(OptionType));

/** 行权方式。 */
export const ExerciseStyle = {
  /** 欧式：仅到期可行权。 */
  European: 'european',
  /** 美式：到期前任意时间可行权。 */
  American: 'american',
} as const;

/** 行权方式联合类型。 */
export type ExerciseStyle = (typeof ExerciseStyle)[keyof typeof ExerciseStyle];

/** 行权方式集合。 */
const EXERCISE_STYLES: readonly ExerciseStyle[] = Object.freeze(Object.values(ExerciseStyle));

/** venue 规范：小写字母/数字开头，最长 32。 */
const VENUE_PATTERN = /^[a-z0-9][a-z0-9._-]{0,31}$/;

/** symbol 规范：大写字母/数字开头，允许 `._/-`，最长 64。 */
const SYMBOL_PATTERN = /^[A-Z0-9][A-Z0-9._/-]{0,63}$/;

/** 资产名规范：不允许 `/`。 */
const ASSET_PATTERN = /^[A-Z0-9][A-Z0-9._-]{0,63}$/;

/** 标的公共输入。 */
export interface InstrumentBaseInput {
  /** 交易场所，例如 binance、nasdaq；会转小写。 */
  readonly venue: string;
  /** 场所内代码，例如 BTC/USDT、AAPL、ES；会转大写。 */
  readonly symbol: string;
  /** 计价货币。 */
  readonly quoteCurrency: Currency;
  /** 最小价格变动单位，必须 > 0。 */
  readonly tickSize: Decimal;
  /** 最小数量变动单位，必须 > 0。 */
  readonly lotSize: Decimal;
  /** 结算货币；v0.1 必须与 quoteCurrency 完全一致，缺省取其值。 */
  readonly settlementCurrency?: Currency;
  /** 合约乘数，缺省 1。 */
  readonly contractMultiplier?: Decimal;
  /** 最小下单数量；若提供必须是 lotSize 的整数倍。 */
  readonly minQuantity?: Decimal;
  /** 最大下单数量；若提供必须是 lotSize 的整数倍。 */
  readonly maxQuantity?: Decimal;
  /** 最小名义价值；币种必须等于 quoteCurrency。 */
  readonly minNotional?: Money;
  /** 是否可交易，缺省 true。 */
  readonly isTradable?: boolean;
}

/** 现货/股票/加密/指数输入。 */
export interface SpotInstrumentInput extends InstrumentBaseInput {
  /** 资产类别。 */
  readonly assetClass: 'equity' | 'crypto' | 'index';
  /** 基础资产，例如 AAPL、BTC、SPX。 */
  readonly baseAsset: string;
}

/** FX 输入。 */
export interface FxInstrumentInput extends InstrumentBaseInput {
  /** 资产类别。 */
  readonly assetClass: 'fx';
  /** 基础货币；不得与 quoteCurrency 相同。 */
  readonly baseCurrency: Currency;
}

/** 期货输入。 */
export interface FutureInstrumentInput extends InstrumentBaseInput {
  /** 资产类别。 */
  readonly assetClass: 'future';
  /** 到期时间（epoch 毫秒）。 */
  readonly expiryMs: number;
  /** 结算方式。 */
  readonly settlement: SettlementType;
}

/** 永续输入。 */
export interface PerpetualInstrumentInput extends InstrumentBaseInput {
  /** 资产类别。 */
  readonly assetClass: 'perpetual';
  /** 资金费间隔（毫秒），必须 > 0。 */
  readonly fundingIntervalMs: number;
}

/** 期权输入。 */
export interface OptionInstrumentInput extends InstrumentBaseInput {
  /** 资产类别。 */
  readonly assetClass: 'option';
  /** call / put。 */
  readonly optionType: OptionType;
  /** 行权价，必须 > 0。 */
  readonly strike: Decimal;
  /** 到期时间（epoch 毫秒）。 */
  readonly expiryMs: number;
  /** 行权方式。 */
  readonly exerciseStyle: ExerciseStyle;
  /** 结算方式。 */
  readonly settlement: SettlementType;
  /** 标的代码，例如 BTC/USDT 或 SPX。 */
  readonly underlying: string;
}

/** 标的创建输入。 */
export type InstrumentInput =
  | SpotInstrumentInput
  | FxInstrumentInput
  | FutureInstrumentInput
  | PerpetualInstrumentInput
  | OptionInstrumentInput;

/** 标的公共字段。 */
export interface InstrumentBase {
  /** 规范化后的交易场所。 */
  readonly venue: string;
  /** 规范化后的代码。 */
  readonly symbol: string;
  /** 计价货币。 */
  readonly quoteCurrency: Currency;
  /** 结算货币；v0.1 与计价货币一致。 */
  readonly settlementCurrency: Currency;
  /** 最小价格变动。 */
  readonly tickSize: Decimal;
  /** 最小数量变动。 */
  readonly lotSize: Decimal;
  /** 合约乘数。 */
  readonly contractMultiplier: Decimal;
  /** 最小下单数量。 */
  readonly minQuantity?: Decimal;
  /** 最大下单数量。 */
  readonly maxQuantity?: Decimal;
  /** 最小名义价值。 */
  readonly minNotional?: Money;
  /** 是否可交易。 */
  readonly isTradable: boolean;
}

/** 现货/股票/加密/指数标的。 */
export interface SpotInstrument extends InstrumentBase {
  /** 资产类别。 */
  readonly assetClass: 'equity' | 'crypto' | 'index';
  /** 基础资产。 */
  readonly baseAsset: string;
}

/** FX 标的。 */
export interface FxInstrument extends InstrumentBase {
  /** 资产类别。 */
  readonly assetClass: 'fx';
  /** 基础货币。 */
  readonly baseCurrency: Currency;
}

/** 期货标的。 */
export interface FutureInstrument extends InstrumentBase {
  /** 资产类别。 */
  readonly assetClass: 'future';
  /** 到期时间（epoch 毫秒）。 */
  readonly expiryMs: number;
  /** 结算方式。 */
  readonly settlement: SettlementType;
}

/** 永续标的。 */
export interface PerpetualInstrument extends InstrumentBase {
  /** 资产类别。 */
  readonly assetClass: 'perpetual';
  /** 资金费间隔（毫秒）。 */
  readonly fundingIntervalMs: number;
}

/** 期权标的。 */
export interface OptionInstrument extends InstrumentBase {
  /** 资产类别。 */
  readonly assetClass: 'option';
  /** call / put。 */
  readonly optionType: OptionType;
  /** 行权价。 */
  readonly strike: Decimal;
  /** 到期时间（epoch 毫秒）。 */
  readonly expiryMs: number;
  /** 行权方式。 */
  readonly exerciseStyle: ExerciseStyle;
  /** 结算方式。 */
  readonly settlement: SettlementType;
  /** 标的代码。 */
  readonly underlying: string;
}

/** 金融标的判别联合。 */
export type Instrument =
  SpotInstrument | FxInstrument | FutureInstrument | PerpetualInstrument | OptionInstrument;

/** 规范化后的公共字段。 */
interface NormalizedBase {
  readonly venue: string;
  readonly symbol: string;
  readonly quoteCurrency: Currency;
  readonly settlementCurrency: Currency;
  readonly tickSize: Decimal;
  readonly lotSize: Decimal;
  readonly contractMultiplier: Decimal;
  readonly minQuantity?: Decimal;
  readonly maxQuantity?: Decimal;
  readonly minNotional?: Money;
  readonly isTradable: boolean;
}

/**
 * 创建金融标的。
 *
 * @param input - 判别联合输入
 * @returns 冻结标的对象
 * @throws {FinanceDefinitionError} 任一定义非法
 * @throws {CurrencyMismatchError} settlementCurrency 与 quoteCurrency 不一致
 *
 * @example
 * ```ts
 * const btc = createInstrument({
 *   assetClass: 'crypto',
 *   venue: 'binance',
 *   symbol: 'BTC/USDT',
 *   baseAsset: 'BTC',
 *   quoteCurrency: createCurrency('USDT', 8),
 *   tickSize: Decimal.parse('0.01'),
 *   lotSize: Decimal.parse('0.0001'),
 * });
 * ```
 */
export function createInstrument(input: InstrumentInput): Instrument {
  if (!isPlainObject(input)) {
    throw new FinanceDefinitionError(`instrument 必须是普通对象（实际值: ${String(input)}）`);
  }
  const base = normalizeBase(input);

  switch (input.assetClass) {
    case AssetClass.Equity:
    case AssetClass.Crypto:
    case AssetClass.Index: {
      const baseAsset = normalizeAsset(input.baseAsset, 'instrument.baseAsset');
      return Object.freeze({ ...base, assetClass: input.assetClass, baseAsset });
    }
    case AssetClass.Fx: {
      if (!isCurrency(input.baseCurrency)) {
        throw new FinanceDefinitionError('instrument.baseCurrency 非法');
      }
      if (input.baseCurrency.code === base.quoteCurrency.code) {
        throw new FinanceDefinitionError(
          `FX baseCurrency 与 quoteCurrency 不能相同（${input.baseCurrency.code}）`,
        );
      }
      return Object.freeze({
        ...base,
        assetClass: AssetClass.Fx,
        baseCurrency: input.baseCurrency,
      });
    }
    case AssetClass.Future: {
      assertPositiveSafeInteger(input.expiryMs, 'instrument.expiryMs');
      assertOneOf(input.settlement, SETTLEMENT_TYPES, 'instrument.settlement');
      return Object.freeze({
        ...base,
        assetClass: AssetClass.Future,
        expiryMs: input.expiryMs,
        settlement: input.settlement,
      });
    }
    case AssetClass.Perpetual: {
      assertPositiveSafeInteger(input.fundingIntervalMs, 'instrument.fundingIntervalMs');
      return Object.freeze({
        ...base,
        assetClass: AssetClass.Perpetual,
        fundingIntervalMs: input.fundingIntervalMs,
      });
    }
    case AssetClass.Option: {
      assertOneOf(input.optionType, OPTION_TYPES, 'instrument.optionType');
      assertPositiveDecimal(input.strike, 'instrument.strike');
      assertPositiveSafeInteger(input.expiryMs, 'instrument.expiryMs');
      assertOneOf(input.exerciseStyle, EXERCISE_STYLES, 'instrument.exerciseStyle');
      assertOneOf(input.settlement, SETTLEMENT_TYPES, 'instrument.settlement');
      const underlying = normalizeSymbol(input.underlying, 'instrument.underlying');
      return Object.freeze({
        ...base,
        assetClass: AssetClass.Option,
        optionType: input.optionType,
        strike: input.strike,
        expiryMs: input.expiryMs,
        exerciseStyle: input.exerciseStyle,
        settlement: input.settlement,
        underlying,
      });
    }
    default: {
      const exhaustive: never = input;
      throw new FinanceDefinitionError(
        `未知 assetClass: ${String((exhaustive as { assetClass?: unknown }).assetClass)}`,
      );
    }
  }
}

/** 校验并规范化公共字段。 */
function normalizeBase(input: InstrumentBaseInput): NormalizedBase {
  const venue = normalizeVenue(input.venue);
  const symbol = normalizeSymbol(input.symbol, 'instrument.symbol');
  if (!isCurrency(input.quoteCurrency)) {
    throw new FinanceDefinitionError('instrument.quoteCurrency 非法');
  }
  const quoteCurrency = input.quoteCurrency;
  const settlementCurrency = input.settlementCurrency ?? quoteCurrency;
  if (!isCurrency(settlementCurrency)) {
    throw new FinanceDefinitionError('instrument.settlementCurrency 非法');
  }
  if (!currencyEquals(quoteCurrency, settlementCurrency)) {
    throw new CurrencyMismatchError(
      currencyLabel(quoteCurrency),
      currencyLabel(settlementCurrency),
    );
  }

  assertPositiveDecimal(input.tickSize, 'instrument.tickSize');
  assertPositiveDecimal(input.lotSize, 'instrument.lotSize');
  const contractMultiplier = input.contractMultiplier ?? Decimal.ONE;
  assertPositiveDecimal(contractMultiplier, 'instrument.contractMultiplier');

  const minQuantity = input.minQuantity;
  const maxQuantity = input.maxQuantity;
  if (minQuantity !== undefined) {
    assertPositiveDecimal(minQuantity, 'instrument.minQuantity');
    assertLotMultiple(minQuantity, input.lotSize, 'instrument.minQuantity');
  }
  if (maxQuantity !== undefined) {
    assertPositiveDecimal(maxQuantity, 'instrument.maxQuantity');
    assertLotMultiple(maxQuantity, input.lotSize, 'instrument.maxQuantity');
  }
  if (
    minQuantity !== undefined &&
    maxQuantity !== undefined &&
    minQuantity.compare(maxQuantity) > 0
  ) {
    throw new FinanceDefinitionError(
      `instrument.minQuantity ${minQuantity.toString()} 不能大于 maxQuantity ${maxQuantity.toString()}`,
    );
  }

  const minNotional = input.minNotional;
  if (minNotional !== undefined) {
    if (!isMoney(minNotional)) {
      throw new FinanceDefinitionError('instrument.minNotional 非法');
    }
    if (!currencyEquals(minNotional.currency, quoteCurrency)) {
      throw new CurrencyMismatchError(
        currencyLabel(quoteCurrency),
        currencyLabel(minNotional.currency),
      );
    }
    if (!minNotional.amount.isPositive()) {
      throw new FinanceDefinitionError('instrument.minNotional 必须大于 0');
    }
  }

  if (input.isTradable !== undefined) {
    assertBoolean(input.isTradable, 'instrument.isTradable');
  }

  return {
    venue,
    symbol,
    quoteCurrency,
    settlementCurrency,
    tickSize: input.tickSize,
    lotSize: input.lotSize,
    contractMultiplier,
    ...(minQuantity === undefined ? {} : { minQuantity }),
    ...(maxQuantity === undefined ? {} : { maxQuantity }),
    ...(minNotional === undefined ? {} : { minNotional }),
    isTradable: input.isTradable ?? true,
  };
}

/** 校验数量是 lot 的整数倍。 */
function assertLotMultiple(quantity: Decimal, lotSize: Decimal, path: string): void {
  if (!isDecimalMultipleOf(quantity, lotSize)) {
    throw new FinanceDefinitionError(
      `${path} 必须是 lotSize ${lotSize.toString()} 的整数倍（实际值: ${quantity.toString()}）`,
    );
  }
}

/** 规范化 venue。 */
function normalizeVenue(value: string): string {
  assertNonEmptyString(value, 'instrument.venue');
  const normalized = value.trim().toLowerCase();
  assertPattern(normalized, VENUE_PATTERN, 'instrument.venue');
  return normalized;
}

/** 规范化 symbol/underlying。 */
function normalizeSymbol(value: string, path: string): string {
  assertNonEmptyString(value, path);
  const normalized = value.trim().toUpperCase();
  assertPattern(normalized, SYMBOL_PATTERN, path);
  return normalized;
}

/** 规范化基础资产。 */
function normalizeAsset(value: string, path: string): string {
  assertNonEmptyString(value, path);
  const normalized = value.trim().toUpperCase();
  assertPattern(normalized, ASSET_PATTERN, path);
  return normalized;
}

/**
 * 判断值是否为合法标的。
 *
 * @param value - 待判断值
 * @returns 结构合法时 true
 */
export function isInstrument(value: unknown): value is Instrument {
  if (!isPlainObject(value)) return false;
  const assetClass = value['assetClass'];
  if (typeof assetClass !== 'string' || !ASSET_CLASSES.includes(assetClass as AssetClass)) {
    return false;
  }
  const quoteCurrency = value['quoteCurrency'];
  const settlementCurrency = value['settlementCurrency'];
  if (!isCurrency(quoteCurrency) || !isCurrency(settlementCurrency)) return false;
  if (!currencyEquals(quoteCurrency, settlementCurrency)) return false;
  if (!isDecimal(value['tickSize']) || !value['tickSize'].isPositive()) return false;
  const lotSize = value['lotSize'];
  if (!isDecimal(lotSize) || !lotSize.isPositive()) return false;
  if (!isDecimal(value['contractMultiplier']) || !value['contractMultiplier'].isPositive()) {
    return false;
  }
  if (typeof value['venue'] !== 'string' || !VENUE_PATTERN.test(value['venue'])) return false;
  if (typeof value['symbol'] !== 'string' || !SYMBOL_PATTERN.test(value['symbol'])) return false;
  if (typeof value['isTradable'] !== 'boolean') return false;

  const minQuantity = value['minQuantity'];
  if (minQuantity !== undefined) {
    if (!isDecimal(minQuantity) || !minQuantity.isPositive()) return false;
    if (!isDecimalMultipleOf(minQuantity, lotSize)) return false;
  }
  const maxQuantity = value['maxQuantity'];
  if (maxQuantity !== undefined) {
    if (!isDecimal(maxQuantity) || !maxQuantity.isPositive()) return false;
    if (!isDecimalMultipleOf(maxQuantity, lotSize)) return false;
  }
  if (minQuantity !== undefined && maxQuantity !== undefined) {
    if (!isDecimal(minQuantity) || !isDecimal(maxQuantity)) return false;
    if (minQuantity.compare(maxQuantity) > 0) return false;
  }

  const minNotional = value['minNotional'];
  if (minNotional !== undefined && !isMoney(minNotional)) return false;
  if (minNotional !== undefined) {
    if (!currencyEquals(minNotional.currency, quoteCurrency)) return false;
    if (!minNotional.amount.isPositive()) return false;
  }
  if (
    assetClass === AssetClass.Equity ||
    assetClass === AssetClass.Crypto ||
    assetClass === AssetClass.Index
  ) {
    return typeof value['baseAsset'] === 'string' && ASSET_PATTERN.test(value['baseAsset']);
  }
  if (assetClass === AssetClass.Fx) {
    return isCurrency(value['baseCurrency']) && value['baseCurrency'].code !== quoteCurrency.code;
  }
  if (assetClass === AssetClass.Future) {
    return (
      isPositiveSafeInteger(value['expiryMs']) &&
      typeof value['settlement'] === 'string' &&
      SETTLEMENT_TYPES.includes(value['settlement'] as SettlementType)
    );
  }
  if (assetClass === AssetClass.Perpetual) {
    return isPositiveSafeInteger(value['fundingIntervalMs']);
  }
  return (
    typeof value['optionType'] === 'string' &&
    OPTION_TYPES.includes(value['optionType'] as OptionType) &&
    isDecimal(value['strike']) &&
    value['strike'].isPositive() &&
    isPositiveSafeInteger(value['expiryMs']) &&
    typeof value['exerciseStyle'] === 'string' &&
    EXERCISE_STYLES.includes(value['exerciseStyle'] as ExerciseStyle) &&
    typeof value['settlement'] === 'string' &&
    SETTLEMENT_TYPES.includes(value['settlement'] as SettlementType) &&
    typeof value['underlying'] === 'string' &&
    SYMBOL_PATTERN.test(value['underlying'])
  );
}

/** 判断是否为正安全整数。 */
function isPositiveSafeInteger(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value > 0;
}

/**
 * 生成标的唯一 key。
 *
 * @param instrument - 标的
 * @returns `${venue}:${symbol}`
 */
export function instrumentKey(instrument: Instrument): string {
  return `${instrument.venue}:${instrument.symbol}`;
}

/**
 * 判断两个标的是否为同一 key。
 *
 * @param left - 左标的
 * @param right - 右标的
 * @returns key 相同时 true
 */
export function sameInstrument(left: Instrument, right: Instrument): boolean {
  return instrumentKey(left) === instrumentKey(right);
}

/**
 * 断言两个标的一致。
 *
 * @param expected - 期望标的
 * @param actual - 实际标的
 * @throws {InstrumentMismatchError} key 不一致
 */
export function assertSameInstrument(expected: Instrument, actual: Instrument): void {
  if (!sameInstrument(expected, actual)) {
    throw new InstrumentMismatchError(instrumentKey(expected), instrumentKey(actual));
  }
}

/**
 * 判断是否为现金结算衍生品（期货/永续）。
 *
 * @param instrument - 标的
 * @returns future/perpetual 时 true
 */
export function isCashSettledDerivative(instrument: Instrument): boolean {
  return (
    instrument.assetClass === AssetClass.Future || instrument.assetClass === AssetClass.Perpetual
  );
}

/**
 * 判断是否为按市值持有/支付溢价的标的（现货、FX、指数、期权）。
 *
 * @param instrument - 标的
 * @returns equity/crypto/fx/index/option 时 true
 */
export function isPremiumInstrument(instrument: Instrument): boolean {
  return !isCashSettledDerivative(instrument);
}

/**
 * 判断价格是否合法：> 0 且为 tickSize 的整数倍。
 *
 * @param instrument - 标的
 * @param price - 价格
 * @returns 合法时 true
 */
export function isPriceValid(instrument: Instrument, price: Decimal): boolean {
  return price.isPositive() && isDecimalMultipleOf(price, instrument.tickSize);
}

/**
 * 判断数量是否合法：> 0、为 lotSize 的整数倍并满足 min/max。
 *
 * @param instrument - 标的
 * @param quantity - 数量
 * @returns 合法时 true
 */
export function isQuantityValid(instrument: Instrument, quantity: Decimal): boolean {
  if (!quantity.isPositive() || !isDecimalMultipleOf(quantity, instrument.lotSize)) return false;
  if (instrument.minQuantity !== undefined && quantity.compare(instrument.minQuantity) < 0) {
    return false;
  }
  if (instrument.maxQuantity !== undefined && quantity.compare(instrument.maxQuantity) > 0) {
    return false;
  }
  return true;
}

/**
 * 断言价格合法。
 *
 * @param instrument - 标的
 * @param price - 价格
 * @param path - 错误字段路径
 * @throws {FinanceDefinitionError} 价格非法
 */
export function assertPriceValid(instrument: Instrument, price: Decimal, path = 'price'): void {
  if (!isPriceValid(instrument, price)) {
    throw new FinanceDefinitionError(
      `${path} 必须是 tickSize ${instrument.tickSize.toString()} 的正整数倍（实际值: ${price.toString()}）`,
    );
  }
}

/**
 * 断言数量合法。
 *
 * @param instrument - 标的
 * @param quantity - 数量
 * @param path - 错误字段路径
 * @throws {FinanceDefinitionError} 数量非法
 */
export function assertQuantityValid(
  instrument: Instrument,
  quantity: Decimal,
  path = 'quantity',
): void {
  if (!isQuantityValid(instrument, quantity)) {
    throw new FinanceDefinitionError(
      `${path} 必须满足 lotSize ${instrument.lotSize.toString()}、minQuantity 与 maxQuantity 约束（实际值: ${quantity.toString()}）`,
    );
  }
}

/**
 * 按 tickSize 对齐价格。
 *
 * @param instrument - 标的
 * @param price - 价格
 * @param rounding - 舍入模式，缺省 `half-even`
 * @returns tickSize 整数倍
 */
export function roundPrice(
  instrument: Instrument,
  price: Decimal,
  rounding: DecimalRounding = 'half-even',
): Decimal {
  return roundDecimalToStep(price, instrument.tickSize, rounding);
}

/**
 * 按 lotSize 对齐数量。
 *
 * @param instrument - 标的
 * @param quantity - 数量
 * @param rounding - 舍入模式，缺省 `down`（不放大用户请求）
 * @returns lotSize 整数倍
 */
export function roundQuantity(
  instrument: Instrument,
  quantity: Decimal,
  rounding: DecimalRounding = 'down',
): Decimal {
  return roundDecimalToStep(quantity, instrument.lotSize, rounding);
}

/**
 * 计算名义价值。
 *
 * @param instrument - 标的
 * @param price - 价格，必须 > 0
 * @param quantity - 数量，必须 >= 0
 * @returns `price * quantity * contractMultiplier` 的 quote 金额
 * @throws {FinanceDefinitionError} 参数非法或 scale 溢出
 */
export function notionalMoney(instrument: Instrument, price: Decimal, quantity: Decimal): Money {
  assertPositiveDecimal(price, 'price');
  if (quantity.isNegative()) {
    throw new FinanceDefinitionError(`quantity 不能为负（实际值: ${quantity.toString()}）`);
  }
  return createMoney(
    price.multiply(quantity).multiply(instrument.contractMultiplier),
    instrument.quoteCurrency,
  );
}

/**
 * 计算单位合约价值。
 *
 * @param instrument - 标的
 * @param price - 价格
 * @returns `price * contractMultiplier` 的 quote 金额
 */
export function contractValue(instrument: Instrument, price: Decimal): Money {
  assertPositiveDecimal(price, 'price');
  return createMoney(price.multiply(instrument.contractMultiplier), instrument.quoteCurrency);
}

/** 标的注册表。 */
export interface InstrumentRegistry {
  /** 按 key 查找。 */
  get(key: string): Instrument | undefined;
  /** 按 key 查找，缺失时抛错。 */
  require(key: string): Instrument;
  /** 冻结的标的全集。 */
  list(): readonly Instrument[];
}

/**
 * 创建标的注册表。
 *
 * @param instruments - 标的列表
 * @returns 注册表；重复 key 会失败
 * @throws {FinanceDefinitionError} 标的非法或 key 重复
 */
export function createInstrumentRegistry(instruments: readonly Instrument[]): InstrumentRegistry {
  const map = new Map<string, Instrument>();
  for (const instrument of instruments) {
    if (!isInstrument(instrument)) {
      throw new FinanceDefinitionError('createInstrumentRegistry 收到非法标的');
    }
    const key = instrumentKey(instrument);
    if (map.has(key)) {
      throw new FinanceDefinitionError(`标的 key 重复: ${key}`);
    }
    map.set(key, instrument);
  }
  const frozen = Object.freeze([...map.values()]);

  return Object.freeze({
    get: (key: string) => map.get(key),
    require: (key: string) => {
      const instrument = map.get(key);
      if (instrument === undefined) {
        throw new FinanceDefinitionError(`标的不存在: ${key}`);
      }
      return instrument;
    },
    list: () => frozen,
  });
}
