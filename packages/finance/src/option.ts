/**
 * @file 期权域封装：把期权标的与金额映射到 `@kurumi/math` 的定价数学。
 * @description 定价、Greeks 与隐含波动率由 math 包提供；本文件只负责标的/货币边界。
 * @module finance/option
 */

import {
  Decimal,
  blackScholes,
  blackScholesPrice,
  impliedVolatility,
  optionBreakeven as mathOptionBreakeven,
  optionIntrinsicValue as mathOptionIntrinsicValue,
  optionIsInTheMoney as mathOptionIsInTheMoney,
  optionMoneyness as mathOptionMoneyness,
  type BlackScholesInput,
  type BlackScholesResult,
  type ImpliedVolatilityInput,
  type ImpliedVolatilityOptions,
} from '@kurumi/math';

import { FinanceDefinitionError, FinanceValueError } from './errors.js';
import { isInstrument, type OptionInstrument } from './instrument.js';
import { createMoney, type Money } from './money.js';
import { assertNonNegativeDecimal, assertPositiveDecimal } from './validation.js';

export {
  blackScholes,
  blackScholesPrice,
  impliedVolatility,
  type BlackScholesInput,
  type BlackScholesResult,
  type ImpliedVolatilityInput,
  type ImpliedVolatilityOptions,
};

/**
 * 计算期权内在价值（每单位标的）。
 *
 * @param option - 期权标的
 * @param underlyingPrice - 标的现价，必须 > 0
 * @returns call 为 `max(S-K,0)`，put 为 `max(K-S,0)`
 * @throws {FinanceDefinitionError} option 非法
 * @throws {FinanceValueError} underlyingPrice 非正
 */
export function optionIntrinsicValue(option: OptionInstrument, underlyingPrice: Decimal): Decimal {
  assertOptionInstrument(option);
  if (!underlyingPrice.isPositive()) {
    throw new FinanceValueError(
      `underlyingPrice 必须大于 0（实际值: ${underlyingPrice.toString()}）`,
    );
  }
  return mathOptionIntrinsicValue(option.optionType, underlyingPrice, option.strike);
}

/**
 * 计算期权内在价值（每张合约，含 multiplier）。
 *
 * @param option - 期权标的
 * @param underlyingPrice - 标的现价
 * @returns quote 货币金额
 */
export function optionIntrinsicValueMoney(
  option: OptionInstrument,
  underlyingPrice: Decimal,
): Money {
  return createMoney(
    optionIntrinsicValue(option, underlyingPrice).multiply(option.contractMultiplier),
    option.quoteCurrency,
  );
}

/**
 * 计算期权货币性。
 *
 * @param option - 期权标的
 * @param underlyingPrice - 标的现价
 * @returns `underlyingPrice / strike`，scale 8、half-even
 */
export function optionMoneyness(option: OptionInstrument, underlyingPrice: Decimal): Decimal {
  assertOptionInstrument(option);
  assertPositiveDecimal(underlyingPrice, 'underlyingPrice');
  return mathOptionMoneyness(underlyingPrice, option.strike);
}

/**
 * 判断期权是否实值。
 *
 * @param option - 期权标的
 * @param underlyingPrice - 标的现价
 * @returns call 在 S > K、put 在 S < K 时 true
 */
export function optionIsInTheMoney(option: OptionInstrument, underlyingPrice: Decimal): boolean {
  assertOptionInstrument(option);
  assertPositiveDecimal(underlyingPrice, 'underlyingPrice');
  return mathOptionIsInTheMoney(option.optionType, underlyingPrice, option.strike);
}

/**
 * 计算盈亏平衡点。
 *
 * @param option - 期权标的
 * @param premium - 每单位标的的期权费，必须 >= 0
 * @returns call 为 `strike + premium`，put 为 `strike - premium`
 */
export function optionBreakeven(option: OptionInstrument, premium: Decimal): Decimal {
  assertOptionInstrument(option);
  assertNonNegativeDecimal(premium, 'premium');
  return mathOptionBreakeven(option.optionType, option.strike, premium);
}

/**
 * 计算欧式期权到期损益（每张合约）。
 *
 * @param option - 期权标的
 * @param settlementPrice - 到期结算价
 * @returns 内在价值 * multiplier 的 quote 金额
 */
export function optionPayoffAtExpiry(option: OptionInstrument, settlementPrice: Decimal): Money {
  return optionIntrinsicValueMoney(option, settlementPrice);
}

/** 校验期权标的。 */
function assertOptionInstrument(option: OptionInstrument): void {
  if (!isInstrument(option) || option.assetClass !== 'option') {
    throw new FinanceDefinitionError('option 必须是合法期权标的');
  }
}
