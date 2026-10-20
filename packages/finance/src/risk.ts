/**
 * @file 下单前风控、组合风险、保证金与强平价格估算。
 * @description 风控检查返回完整违规列表；估值失败（缺 mark/FX）仍显式抛错，不静默跳过。
 * @module finance/risk
 */

import { Decimal } from '@kurumi/math';
import { FinanceDefinitionError, FinanceValueError } from './errors.js';
import {
  instrumentKey,
  isInstrument,
  isQuantityValid,
  isPriceValid,
  notionalMoney,
  sameInstrument,
  type Instrument,
} from './instrument.js';
import { compareMoney, createMoney, currencyEquals, currencyLabel, type Money } from './money.js';
import {
  portfolioEquity,
  portfolioGrossExposure,
  type Portfolio,
  type PortfolioValuation,
} from './portfolio.js';
import { OrderSide, type Order } from './order.js';
import type { Position } from './position.js';
import { assertPositiveDecimal } from './validation.js';

/** 风控违规码。 */
export const RiskViolationCode = {
  /** 标的不允许交易。 */
  InstrumentNotTradable: 'instrument_not_tradable',
  /** 标的不在允许集合。 */
  InstrumentNotAllowed: 'instrument_not_allowed',
  /** 数量非法。 */
  InvalidQuantity: 'invalid_quantity',
  /** 价格非法。 */
  InvalidPrice: 'invalid_price',
  /** 订单名义价值超限。 */
  OrderNotionalExceeded: 'order_notional_exceeded',
  /** 持仓数量超限。 */
  PositionQuantityExceeded: 'position_quantity_exceeded',
  /** 持仓名义价值超限。 */
  PositionNotionalExceeded: 'position_notional_exceeded',
  /** 持仓标的不一致。 */
  PositionInstrumentMismatch: 'position_instrument_mismatch',
  /** 限额币种与标的计价币种不一致。 */
  LimitCurrencyMismatch: 'limit_currency_mismatch',
  /** 组合毛敞口超限。 */
  GrossExposureExceeded: 'gross_exposure_exceeded',
  /** 杠杆超限。 */
  LeverageExceeded: 'leverage_exceeded',
  /** 权益低于下限。 */
  MinEquityBreached: 'min_equity_breached',
} as const;

/** 风控违规码联合类型。 */
export type RiskViolationCode = (typeof RiskViolationCode)[keyof typeof RiskViolationCode];

/** 单条风控违规。 */
export interface RiskViolation {
  /** 违规码。 */
  readonly code: RiskViolationCode;
  /** 人类可读描述。 */
  readonly message: string;
  /** 限额描述。 */
  readonly limit?: string;
  /** 实际值描述。 */
  readonly actual?: string;
}

/** 下单风控限额。 */
export interface RiskLimits {
  /** 单笔订单最大名义价值。 */
  readonly maxOrderNotional?: Money;
  /** 单标的最大绝对持仓数量。 */
  readonly maxPositionQuantity?: Decimal;
  /** 单标的最大绝对持仓名义价值。 */
  readonly maxPositionNotional?: Money;
  /** 允许交易的标的 key 集合；缺省不限制。 */
  readonly allowedInstrumentKeys?: readonly string[];
}

/** 下单风控上下文。 */
export interface OrderRiskContext {
  /** 标的。 */
  readonly instrument: Instrument;
  /** 方向。 */
  readonly side: OrderSide;
  /** 数量。 */
  readonly quantity: Decimal;
  /** 参考价格。 */
  readonly price: Decimal;
  /** 当前持仓；缺省视为空仓。 */
  readonly position?: Position;
  /** 限额。 */
  readonly limits: RiskLimits;
}

/** 组合风控限额。 */
export interface PortfolioRiskLimits {
  /** 最大毛敞口（base 货币）。 */
  readonly maxGrossExposure?: Money;
  /** 最大杠杆（毛敞口 / 权益）。 */
  readonly maxLeverage?: Decimal;
  /** 最小权益（base 货币）。 */
  readonly minEquity?: Money;
}

/** 组合风控上下文。 */
export interface PortfolioRiskContext {
  /** 组合。 */
  readonly portfolio: Portfolio;
  /** 估值输入。 */
  readonly valuation: PortfolioValuation;
  /** 限额。 */
  readonly limits: PortfolioRiskLimits;
}

/** 保证金费率。 */
export interface MarginRates {
  /** 初始保证金率，`0..1`。 */
  readonly initial: Decimal;
  /** 维持保证金率，`0..1`，且不大于 initial。 */
  readonly maintenance: Decimal;
}

/** 保证金要求。 */
export interface MarginRequirement {
  /** 名义价值。 */
  readonly notional: Money;
  /** 初始保证金。 */
  readonly initialMargin: Money;
  /** 维持保证金。 */
  readonly maintenanceMargin: Money;
}

/**
 * 下单前风控检查。
 *
 * @param context - 标的、方向、数量、价格、持仓与限额
 * @returns 按固定顺序排列的全部违规；无违规时为空数组
 */
export function checkOrderRisk(context: OrderRiskContext): readonly RiskViolation[] {
  const violations: RiskViolation[] = [];
  const instrument = context.instrument;
  if (!isInstrument(instrument)) {
    return Object.freeze([violation(RiskViolationCode.InstrumentNotAllowed, '标的结构非法')]);
  }
  if (!instrument.isTradable) {
    violations.push(
      violation(
        RiskViolationCode.InstrumentNotTradable,
        `标的不允许交易: ${instrumentKey(instrument)}`,
      ),
    );
  }
  if (
    context.limits.allowedInstrumentKeys !== undefined &&
    !context.limits.allowedInstrumentKeys.includes(instrumentKey(instrument))
  ) {
    violations.push(
      violation(
        RiskViolationCode.InstrumentNotAllowed,
        `标的不在允许集合: ${instrumentKey(instrument)}`,
        context.limits.allowedInstrumentKeys.join(','),
        instrumentKey(instrument),
      ),
    );
  }

  const quantityIsValid = isQuantityValid(instrument, context.quantity);
  if (!quantityIsValid) {
    violations.push(
      violation(
        RiskViolationCode.InvalidQuantity,
        `数量不满足 lot/min/max 约束（实际值: ${context.quantity.toString()}）`,
      ),
    );
  }

  const priceIsValid = isPriceValid(instrument, context.price);
  if (!priceIsValid) {
    violations.push(
      violation(
        RiskViolationCode.InvalidPrice,
        `价格不是 tickSize 的正整数倍（实际值: ${context.price.toString()}）`,
      ),
    );
  }

  if (!quantityIsValid || !priceIsValid) {
    return Object.freeze(violations);
  }

  const orderNotional = notionalMoney(instrument, context.price, context.quantity);
  checkMoneyLimit(
    violations,
    RiskViolationCode.OrderNotionalExceeded,
    orderNotional,
    context.limits.maxOrderNotional,
    '订单名义价值',
  );

  const signedQuantity =
    context.side === OrderSide.Buy ? context.quantity : context.quantity.negate();
  const currentPosition = context.position;
  if (currentPosition !== undefined && !sameInstrument(currentPosition.instrument, instrument)) {
    violations.push(
      violation(
        RiskViolationCode.PositionInstrumentMismatch,
        `持仓标的与订单标的不一致（持仓 ${instrumentKey(currentPosition.instrument)}，订单 ${instrumentKey(instrument)}）`,
      ),
    );
    return Object.freeze(violations);
  }

  const currentNet = currentPosition?.netQuantity ?? Decimal.ZERO;
  const resultingNet = currentNet.add(signedQuantity);
  const maxPositionQuantity = context.limits.maxPositionQuantity;
  if (maxPositionQuantity !== undefined && resultingNet.abs().compare(maxPositionQuantity) > 0) {
    violations.push(
      violation(
        RiskViolationCode.PositionQuantityExceeded,
        `预计持仓数量 ${resultingNet.abs().toString()} 超过上限 ${maxPositionQuantity.toString()}`,
        maxPositionQuantity.toString(),
        resultingNet.abs().toString(),
      ),
    );
  }

  const maxPositionNotional = context.limits.maxPositionNotional;
  if (maxPositionNotional !== undefined) {
    if (!currencyEquals(maxPositionNotional.currency, instrument.quoteCurrency)) {
      violations.push(
        violation(
          RiskViolationCode.LimitCurrencyMismatch,
          `持仓名义限额币种不一致（限额 ${currencyLabel(maxPositionNotional.currency)}，标的 ${currencyLabel(instrument.quoteCurrency)}）`,
        ),
      );
    } else {
      const resultingNotional = notionalMoney(instrument, context.price, resultingNet.abs());
      if (compareMoney(resultingNotional, maxPositionNotional) > 0) {
        violations.push(
          violation(
            RiskViolationCode.PositionNotionalExceeded,
            `预计持仓名义价值 ${resultingNotional.amount.toString()} 超过上限 ${maxPositionNotional.amount.toString()}`,
            maxPositionNotional.amount.toString(),
            resultingNotional.amount.toString(),
          ),
        );
      }
    }
  }

  return Object.freeze(violations);
}

/**
 * 组合风险检查。
 *
 * @param context - 组合、估值与限额
 * @returns 按固定顺序排列的全部违规
 * @throws {MissingMarkPriceError} 缺少持仓 mark
 * @throws {MissingFxRateError} 缺少跨币种汇率
 */
export function checkPortfolioRisk(context: PortfolioRiskContext): readonly RiskViolation[] {
  const violations: RiskViolation[] = [];
  const baseCurrency = context.portfolio.baseCurrency;
  const equity = portfolioEquity(context.portfolio, context.valuation);
  const grossExposure = portfolioGrossExposure(context.portfolio, context.valuation);

  const maxGrossExposure = context.limits.maxGrossExposure;
  if (maxGrossExposure !== undefined) {
    if (!currencyEquals(maxGrossExposure.currency, baseCurrency)) {
      violations.push(
        violation(
          RiskViolationCode.LimitCurrencyMismatch,
          `毛敞口限额币种不一致（限额 ${currencyLabel(maxGrossExposure.currency)}，基准 ${currencyLabel(baseCurrency)}）`,
        ),
      );
    } else if (compareMoney(grossExposure, maxGrossExposure) > 0) {
      violations.push(
        violation(
          RiskViolationCode.GrossExposureExceeded,
          `毛敞口 ${grossExposure.amount.toString()} 超过上限 ${maxGrossExposure.amount.toString()}`,
          maxGrossExposure.amount.toString(),
          grossExposure.amount.toString(),
        ),
      );
    }
  }

  const maxLeverage = context.limits.maxLeverage;
  if (maxLeverage !== undefined) {
    if (!maxLeverage.isPositive()) {
      violations.push(violation(RiskViolationCode.LeverageExceeded, 'maxLeverage 必须大于 0'));
    } else if (!equity.amount.isPositive()) {
      violations.push(
        violation(
          RiskViolationCode.LeverageExceeded,
          `权益非正（${equity.amount.toString()}），无法满足杠杆上限`,
          maxLeverage.toString(),
          equity.amount.toString(),
        ),
      );
    } else {
      const leverage = grossExposure.amount.divide(equity.amount, {
        scale: 8,
        rounding: 'half-even',
      });
      if (leverage.compare(maxLeverage) > 0) {
        violations.push(
          violation(
            RiskViolationCode.LeverageExceeded,
            `杠杆 ${leverage.toString()} 超过上限 ${maxLeverage.toString()}`,
            maxLeverage.toString(),
            leverage.toString(),
          ),
        );
      }
    }
  }

  const minEquity = context.limits.minEquity;
  if (minEquity !== undefined) {
    if (!currencyEquals(minEquity.currency, baseCurrency)) {
      violations.push(
        violation(
          RiskViolationCode.LimitCurrencyMismatch,
          `最小权益限额币种不一致（限额 ${currencyLabel(minEquity.currency)}，基准 ${currencyLabel(baseCurrency)}）`,
        ),
      );
    } else if (compareMoney(equity, minEquity) < 0) {
      violations.push(
        violation(
          RiskViolationCode.MinEquityBreached,
          `权益 ${equity.amount.toString()} 低于下限 ${minEquity.amount.toString()}`,
          minEquity.amount.toString(),
          equity.amount.toString(),
        ),
      );
    }
  }

  return Object.freeze(violations);
}

/**
 * 计算保证金要求。
 *
 * @param instrument - 标的
 * @param quantity - 数量（取绝对值）
 * @param price - 价格，必须 > 0
 * @param rates - 初始/维持保证金率，均在 `0..1` 且 maintenance <= initial
 * @returns 名义价值与两项保证金
 * @throws {FinanceDefinitionError} 参数非法
 */
export function calculateMarginRequirement(
  instrument: Instrument,
  quantity: Decimal,
  price: Decimal,
  rates: MarginRates,
): MarginRequirement {
  if (!isInstrument(instrument)) {
    throw new FinanceDefinitionError('margin.instrument 非法');
  }
  assertPositiveDecimal(price, 'margin.price');
  if (!(rates.initial instanceof Decimal) || rates.initial.isNegative()) {
    throw new FinanceDefinitionError('margin.initial 必须是非负 Decimal');
  }
  if (rates.initial.compare(Decimal.ONE) > 0) {
    throw new FinanceDefinitionError('margin.initial 不能大于 1');
  }
  if (!(rates.maintenance instanceof Decimal) || rates.maintenance.isNegative()) {
    throw new FinanceDefinitionError('margin.maintenance 必须是非负 Decimal');
  }
  if (rates.maintenance.compare(rates.initial) > 0) {
    throw new FinanceDefinitionError('margin.maintenance 不能大于 initial');
  }
  if (!(quantity instanceof Decimal) || quantity.isNegative()) {
    throw new FinanceDefinitionError('margin.quantity 必须是非负 Decimal');
  }

  const notional = notionalMoney(instrument, price, quantity.abs());
  const initialMargin = createMoney(
    notional.amount.multiply(rates.initial),
    instrument.quoteCurrency,
  );
  const maintenanceMargin = createMoney(
    notional.amount.multiply(rates.maintenance),
    instrument.quoteCurrency,
  );
  return Object.freeze({ notional, initialMargin, maintenanceMargin });
}

/**
 * 估算逐仓强平价格。
 *
 * @param position - 持仓，净数量不能为零
 * @param leverage - 杠杆，必须 > 0
 * @param maintenanceMarginRate - 维持保证金率，`0 <= rate < 1`
 * @returns quote 货币强平价格；多头结果不低于 0
 * @throws {FinanceValueError} 参数非法
 *
 * @example
 * ```ts
 * // 多头 10x、维持保证金率 0.5%：强平价约为开仓价的 90.5%
 * liquidationPrice(longPosition, Decimal.parse('10'), Decimal.parse('0.005'));
 * ```
 */
export function liquidationPrice(
  position: Position,
  leverage: Decimal,
  maintenanceMarginRate: Decimal,
): Money {
  if (position.netQuantity.isZero()) {
    throw new FinanceValueError('liquidationPrice 不接受空仓');
  }
  if (!leverage.isPositive()) {
    throw new FinanceValueError(`leverage 必须大于 0（实际值: ${leverage.toString()}）`);
  }
  if (!maintenanceMarginRate.isPositive() && !maintenanceMarginRate.isZero()) {
    throw new FinanceValueError(
      `maintenanceMarginRate 不能为负（实际值: ${maintenanceMarginRate.toString()}）`,
    );
  }
  if (maintenanceMarginRate.compare(Decimal.ONE) >= 0) {
    throw new FinanceValueError('maintenanceMarginRate 必须小于 1');
  }

  const inverseLeverage = Decimal.ONE.divide(leverage, {
    scale: 18,
    rounding: 'half-even',
  });
  const isLong = position.netQuantity.isPositive();
  const factor = isLong
    ? Decimal.ONE.subtract(inverseLeverage).add(maintenanceMarginRate)
    : Decimal.ONE.add(inverseLeverage).subtract(maintenanceMarginRate);
  const price = position.averageEntryPrice.multiply(factor);
  return createMoney(price.isNegative() ? Decimal.ZERO : price, position.instrument.quoteCurrency);
}

/**
 * 从订单对象执行风控检查。
 *
 * @param order - 订单
 * @param price - 参考价格
 * @param limits - 限额
 * @param position - 当前持仓
 * @returns 违规列表
 */
export function checkOrderRiskForOrder(
  order: Order,
  price: Decimal,
  limits: RiskLimits,
  position?: Position,
): readonly RiskViolation[] {
  return checkOrderRisk({
    instrument: order.instrument,
    side: order.side,
    quantity: order.remainingQuantity,
    price,
    ...(position === undefined ? {} : { position }),
    limits,
  });
}

/** 构造冻结违规对象。 */
function violation(
  code: RiskViolationCode,
  message: string,
  limit?: string,
  actual?: string,
): RiskViolation {
  return Object.freeze({
    code,
    message,
    ...(limit === undefined ? {} : { limit }),
    ...(actual === undefined ? {} : { actual }),
  });
}

/** 检查 Money 限额。 */
function checkMoneyLimit(
  violations: RiskViolation[],
  code: RiskViolationCode,
  actual: Money,
  limit: Money | undefined,
  label: string,
): void {
  if (limit === undefined) return;
  if (!currencyEquals(actual.currency, limit.currency)) {
    violations.push(
      violation(
        RiskViolationCode.LimitCurrencyMismatch,
        `${label}限额币种不一致（限额 ${currencyLabel(limit.currency)}，标的 ${currencyLabel(actual.currency)}）`,
      ),
    );
    return;
  }
  if (compareMoney(actual, limit) > 0) {
    violations.push(
      violation(
        code,
        `${label} ${actual.amount.toString()} 超过上限 ${limit.amount.toString()}`,
        limit.amount.toString(),
        actual.amount.toString(),
      ),
    );
  }
}
