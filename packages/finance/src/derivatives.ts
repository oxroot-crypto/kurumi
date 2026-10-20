/**
 * @file 期货与永续合约衍生指标：基差、基差率与资金费。
 * @description 只处理价格/数量推导；保证金与强平估算位于 `risk.ts`。
 * @module finance/derivatives
 */

import { Decimal } from '@kurumi/math';
import { FinanceDefinitionError, FinanceValueError } from './errors.js';
import { isInstrument, type FutureInstrument, type PerpetualInstrument } from './instrument.js';
import { createMoney, type Money } from './money.js';
import { assertPositiveDecimal } from './validation.js';

/** 资金费输入。 */
export interface FundingPaymentInput {
  /** 永续标的。 */
  readonly instrument: PerpetualInstrument;
  /** 带符号持仓数量：正多负空。 */
  readonly netQuantity: Decimal;
  /** 标记价格，必须 > 0。 */
  readonly markPrice: Decimal;
  /** 本期资金费率；正数表示多头支付空头。 */
  readonly fundingRate: Decimal;
}

/**
 * 计算期货基差。
 *
 * @param futuresPrice - 期货价格，必须 > 0
 * @param spotPrice - 现货价格，必须 > 0
 * @returns `futuresPrice - spotPrice`
 * @throws {FinanceDefinitionError} 价格非法
 */
export function futuresBasis(futuresPrice: Decimal, spotPrice: Decimal): Decimal {
  assertPositiveDecimal(futuresPrice, 'futuresPrice');
  assertPositiveDecimal(spotPrice, 'spotPrice');
  return futuresPrice.subtract(spotPrice);
}

/**
 * 计算期货基差率。
 *
 * @param futuresPrice - 期货价格
 * @param spotPrice - 现货价格
 * @returns `(F - S) / S`，scale 8、half-even
 */
export function futuresBasisRate(futuresPrice: Decimal, spotPrice: Decimal): Decimal {
  return futuresBasis(futuresPrice, spotPrice).divide(spotPrice, {
    scale: 8,
    rounding: 'half-even',
  });
}

/**
 * 判断期货是否升水。
 *
 * @param futuresPrice - 期货价格
 * @param spotPrice - 现货价格
 * @returns futuresPrice > spotPrice 时 true
 */
export function futuresIsContango(futuresPrice: Decimal, spotPrice: Decimal): boolean {
  return futuresBasis(futuresPrice, spotPrice).isPositive();
}

/**
 * 判断期货是否贴水。
 *
 * @param futuresPrice - 期货价格
 * @param spotPrice - 现货价格
 * @returns futuresPrice < spotPrice 时 true
 */
export function futuresIsBackwardation(futuresPrice: Decimal, spotPrice: Decimal): boolean {
  return futuresBasis(futuresPrice, spotPrice).isNegative();
}

/**
 * 计算永续合约资金费。
 *
 * @param input - 永续标的、带符号持仓、标记价与资金费率
 * @returns quote 货币资金费；正数表示收取，负数表示支付
 * @throws {FinanceDefinitionError} 标的不合法
 * @throws {FinanceValueError} markPrice 非正或数量非法
 */
export function fundingPayment(input: FundingPaymentInput): Money {
  if (!isInstrument(input.instrument) || input.instrument.assetClass !== 'perpetual') {
    throw new FinanceDefinitionError('fundingPayment.instrument 必须是永续标的');
  }
  if (!input.markPrice.isPositive()) {
    throw new FinanceValueError(`markPrice 必须大于 0（实际值: ${input.markPrice.toString()}）`);
  }
  if (!(input.fundingRate instanceof Decimal)) {
    throw new FinanceDefinitionError('fundingRate 必须是 Decimal');
  }
  return createMoney(
    input.netQuantity
      .negate()
      .multiply(input.markPrice)
      .multiply(input.instrument.contractMultiplier)
      .multiply(input.fundingRate),
    input.instrument.quoteCurrency,
  );
}

/**
 * 计算期货合约名义价值（便捷入口）。
 *
 * @param instrument - 期货标的
 * @param price - 价格
 * @param quantity - 数量，必须 >= 0
 * @returns quote 货币名义价值
 */
export function futureNotional(
  instrument: FutureInstrument,
  price: Decimal,
  quantity: Decimal,
): Money {
  assertPositiveDecimal(price, 'price');
  if (quantity.isNegative()) {
    throw new FinanceDefinitionError(`quantity 不能为负（实际值: ${quantity.toString()}）`);
  }
  return createMoney(
    price.multiply(quantity).multiply(instrument.contractMultiplier),
    instrument.quoteCurrency,
  );
}
