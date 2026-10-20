/**
 * @file 货币、金额与金额运算。
 * @description 金额由 `Decimal` 与冻结的 `Currency` 组成；同币种运算精确，跨币种必须显式经 FX 换算。
 * @module finance/money
 */

import { isPlainObject } from '@kurumi/core';

import {
  Decimal,
  MAX_DECIMAL_SCALE,
  isDecimal,
  sumDecimals,
  type DecimalDivisionOptions,
  type DecimalMultiplyOptions,
  type DecimalRounding,
} from '@kurumi/math';
import { CurrencyMismatchError, FinanceDefinitionError } from './errors.js';
import {
  assertDecimal,
  assertNonEmptyString,
  assertPlainObject,
  assertRange,
  assertSafeInteger,
} from './validation.js';

/** 货币代码正则：3..12 位大写字母/数字，首字符为字母。 */
const CURRENCY_CODE_PATTERN = /^[A-Z][A-Z0-9]{2,11}$/;

/** 货币定义。 */
export interface Currency {
  /** 大写货币代码，例如 USD、BTC、USDT。 */
  readonly code: string;
  /** 展示/结算小数位，`0..18`。 */
  readonly decimals: number;
}

/** 金额。 */
export interface Money {
  /** 金额数值；可高于货币展示精度。 */
  readonly amount: Decimal;
  /** 货币定义。 */
  readonly currency: Currency;
}

/** 金额 JSON DTO。 */
export interface MoneyJson {
  /** 规范十进制字符串。 */
  readonly amount: string;
  /** 货币代码。 */
  readonly currency: string;
  /** 货币小数位。 */
  readonly decimals: number;
}

/** 金额格式化选项。 */
export interface FormatMoneyOptions {
  /** 小数位，缺省使用货币 decimals；范围 `0..30`。 */
  readonly decimals?: number;
  /** 舍入模式，缺省 `half-even`。 */
  readonly rounding?: DecimalRounding;
  /** 是否附带货币代码，缺省 true。 */
  readonly shouldIncludeCode?: boolean;
}

/**
 * 创建货币定义。
 *
 * @param code - 货币代码；会去空白并转大写
 * @param decimals - 小数位，`0..18` 的整数
 * @returns 冻结货币对象
 * @throws {FinanceDefinitionError} 代码或小数位非法
 *
 * @example
 * ```ts
 * createCurrency('usd', 2); // { code: 'USD', decimals: 2 }
 * ```
 */
export function createCurrency(code: string, decimals: number): Currency {
  assertNonEmptyString(code, 'currency.code');
  const normalized = code.trim().toUpperCase();
  if (!CURRENCY_CODE_PATTERN.test(normalized)) {
    throw new FinanceDefinitionError(
      `currency.code 必须匹配 ${CURRENCY_CODE_PATTERN.source}（实际值: ${code}）`,
    );
  }
  if (!Number.isSafeInteger(decimals) || decimals < 0 || decimals > 18) {
    throw new FinanceDefinitionError(
      `currency.decimals 必须是 0..18 的整数（实际值: ${String(decimals)}）`,
    );
  }
  return Object.freeze({ code: normalized, decimals });
}

/**
 * 判断值是否为合法货币定义。
 *
 * @param value - 待判断值
 * @returns 结构合法时 true
 */
export function isCurrency(value: unknown): value is Currency {
  if (!isPlainObject(value)) return false;
  const code = value['code'];
  const decimals = value['decimals'];
  return (
    typeof code === 'string' &&
    CURRENCY_CODE_PATTERN.test(code) &&
    typeof decimals === 'number' &&
    Number.isSafeInteger(decimals) &&
    decimals >= 0 &&
    decimals <= 18
  );
}

/**
 * 判断两个货币定义是否完全一致。
 *
 * @param left - 左值
 * @param right - 右值
 * @returns code 与 decimals 都相同时 true
 */
export function currencyEquals(left: Currency, right: Currency): boolean {
  return left.code === right.code && left.decimals === right.decimals;
}

/**
 * 生成用于错误信息的货币标签。
 *
 * @param currency - 货币
 * @returns 形如 `USD(2)` 的标签
 */
export function currencyLabel(currency: Currency): string {
  return `${currency.code}(${currency.decimals})`;
}

/**
 * 创建金额。
 *
 * @param amount - 金额数值
 * @param currency - 货币定义
 * @returns 冻结金额对象
 * @throws {FinanceDefinitionError} 参数非法
 */
export function createMoney(amount: Decimal, currency: Currency): Money {
  assertDecimal(amount, 'money.amount');
  if (!isCurrency(currency)) {
    throw new FinanceDefinitionError(`money.currency 非法（实际值: ${String(currency)}）`);
  }
  return Object.freeze({ amount, currency });
}

/**
 * 解析金额字符串。
 *
 * @param text - 十进制字符串
 * @param currency - 货币定义
 * @returns 冻结金额对象
 * @throws {DecimalParseError} 字符串非法
 * @throws {FinanceDefinitionError} 货币非法
 */
export function parseMoney(text: string, currency: Currency): Money {
  return createMoney(Decimal.parse(text), currency);
}

/**
 * 创建零金额。
 *
 * @param currency - 货币定义
 * @returns 金额为 0 的冻结对象
 */
export function zeroMoney(currency: Currency): Money {
  return createMoney(Decimal.ZERO, currency);
}

/**
 * 判断值是否为合法金额。
 *
 * @param value - 待判断值
 * @returns 结构合法时 true
 */
export function isMoney(value: unknown): value is Money {
  if (!isPlainObject(value)) return false;
  return isDecimal(value['amount']) && isCurrency(value['currency']);
}

/**
 * 断言两个金额币种一致。
 *
 * @param left - 左金额
 * @param right - 右金额
 * @throws {CurrencyMismatchError} 币种定义不一致
 */
export function assertSameCurrency(left: Money, right: Money): void {
  if (!currencyEquals(left.currency, right.currency)) {
    throw new CurrencyMismatchError(currencyLabel(left.currency), currencyLabel(right.currency));
  }
}

/**
 * 金额加法。
 *
 * @param left - 左金额
 * @param right - 右金额
 * @returns 新金额，币种取左值
 * @throws {CurrencyMismatchError} 币种不一致
 */
export function addMoney(left: Money, right: Money): Money {
  assertSameCurrency(left, right);
  return createMoney(left.amount.add(right.amount), left.currency);
}

/**
 * 金额减法。
 *
 * @param left - 被减金额
 * @param right - 减数金额
 * @returns 新金额，币种取左值
 * @throws {CurrencyMismatchError} 币种不一致
 */
export function subtractMoney(left: Money, right: Money): Money {
  assertSameCurrency(left, right);
  return createMoney(left.amount.subtract(right.amount), left.currency);
}

/**
 * 金额求和。
 *
 * @param values - 金额列表
 * @param currency - 期望币种；空列表时用于返回零金额
 * @returns 新金额
 * @throws {CurrencyMismatchError} 任一金额币种不一致
 */
export function sumMoney(values: readonly Money[], currency: Currency): Money {
  let total = Decimal.ZERO;
  for (const value of values) {
    if (!currencyEquals(value.currency, currency)) {
      throw new CurrencyMismatchError(currencyLabel(currency), currencyLabel(value.currency));
    }
    total = total.add(value.amount);
  }
  return createMoney(total, currency);
}

/**
 * 金额取反。
 *
 * @param money - 金额
 * @returns 相反数金额
 */
export function negateMoney(money: Money): Money {
  return createMoney(money.amount.negate(), money.currency);
}

/**
 * 金额绝对值。
 *
 * @param money - 金额
 * @returns 非负金额
 */
export function absMoney(money: Money): Money {
  return createMoney(money.amount.abs(), money.currency);
}

/**
 * 金额乘以系数。
 *
 * @param money - 金额
 * @param factor - 系数
 * @param options - 可选 scale/舍入
 * @returns 新金额，币种不变
 */
export function multiplyMoney(
  money: Money,
  factor: Decimal,
  options?: DecimalMultiplyOptions,
): Money {
  return createMoney(money.amount.multiply(factor, options), money.currency);
}

/**
 * 金额除以系数。
 *
 * @param money - 金额
 * @param divisor - 除数，不能为零
 * @param options - 目标 scale/舍入；缺省 scale 18、half-even
 * @returns 新金额，币种不变
 * @throws {DecimalDivisionByZeroError} 除数为零
 */
export function divideMoney(
  money: Money,
  divisor: Decimal,
  options?: DecimalDivisionOptions,
): Money {
  return createMoney(money.amount.divide(divisor, options), money.currency);
}

/**
 * 金额比较。
 *
 * @param left - 左金额
 * @param right - 右金额
 * @returns 负数/零/正数
 * @throws {CurrencyMismatchError} 币种不一致
 */
export function compareMoney(left: Money, right: Money): -1 | 0 | 1 {
  assertSameCurrency(left, right);
  return left.amount.compare(right.amount);
}

/**
 * 金额相等。
 *
 * @param left - 左金额
 * @param right - 右金额
 * @returns 数值相等且币种一致时 true
 */
export function equalsMoney(left: Money, right: Money): boolean {
  return currencyEquals(left.currency, right.currency) && left.amount.equals(right.amount);
}

/**
 * 是否为零金额。
 *
 * @param money - 金额
 * @returns 金额为 0 时 true
 */
export function isZeroMoney(money: Money): boolean {
  return money.amount.isZero();
}

/**
 * 按指定小数位重新缩放金额。
 *
 * @param money - 金额
 * @param decimals - 目标小数位，`0..30`
 * @param rounding - 舍入模式，缺省 `half-even`
 * @returns 新金额
 */
export function rescaleMoney(
  money: Money,
  decimals: number,
  rounding: DecimalRounding = 'half-even',
): Money {
  assertRange(decimals, 'decimals', 0, MAX_DECIMAL_SCALE);
  return createMoney(money.amount.rescale(decimals, rounding), money.currency);
}

/**
 * 格式化金额。
 *
 * @param money - 金额
 * @param options - 小数位、舍入与是否附带货币代码
 * @returns 形如 `123.45 USD` 的文本
 * @throws {FinanceDefinitionError} decimals 越界
 */
export function formatMoney(money: Money, options?: FormatMoneyOptions): string {
  const decimals = options?.decimals ?? money.currency.decimals;
  if (!Number.isSafeInteger(decimals) || decimals < 0 || decimals > MAX_DECIMAL_SCALE) {
    throw new FinanceDefinitionError(
      `formatMoney decimals 必须是 0..${MAX_DECIMAL_SCALE} 的整数（实际值: ${String(decimals)}）`,
    );
  }
  const text = money.amount.toFixed(decimals, options?.rounding ?? 'half-even');
  return options?.shouldIncludeCode === false ? text : `${text} ${money.currency.code}`;
}

/**
 * 金额转 JSON DTO。
 *
 * @param money - 金额
 * @returns `{ amount, currency, decimals }`
 */
export function moneyToJson(money: Money): MoneyJson {
  return Object.freeze({
    amount: money.amount.toString(),
    currency: money.currency.code,
    decimals: money.currency.decimals,
  });
}

/**
 * 从 JSON DTO 还原金额。
 *
 * @param value - 未经信任的 JSON 值
 * @returns 冻结金额对象
 * @throws {FinanceDefinitionError} 结构非法
 * @throws {DecimalParseError} amount 字符串非法
 */
export function moneyFromJson(value: unknown): Money {
  assertPlainObject(value, 'money');
  const amount = value['amount'];
  const currency = value['currency'];
  const decimals = value['decimals'];
  if (typeof amount !== 'string') {
    throw new FinanceDefinitionError(`money.amount 必须是字符串（实际值: ${String(amount)}）`);
  }
  assertNonEmptyString(currency, 'money.currency');
  assertSafeInteger(decimals, 'money.decimals');
  return createMoney(Decimal.parse(amount), createCurrency(currency, decimals));
}

/**
 * 精确求和 Decimal 列表（金额场景的便捷入口）。
 *
 * @param values - Decimal 列表
 * @returns 精确和
 */
export function sumMoneyAmounts(values: readonly Decimal[]): Decimal {
  return sumDecimals(values);
}
