/**
 * @file 金融领域错误体系。
 * @description 复用 core 的 `KurumiError`，为货币、标的、行情、订单、FX、组合与风控提供稳定错误码；小数与期权模型错误由 `@kurumi/math` 提供并在此透传。
 * @module finance/errors
 */

import { KurumiError, type KurumiErrorOptions } from '@kurumi/core';

/** 金融包所有错误的基类。 */
export class FinanceError extends KurumiError {
  /**
   * @param message - 面向人类的错误描述
   * @param options - 错误码与 cause
   */
  constructor(message: string, options?: KurumiErrorOptions) {
    super(message, { code: 'finance_error', ...options });
  }
}

/** 工厂输入、选项或金融定义非法。 */
export class FinanceDefinitionError extends FinanceError {
  /**
   * @param message - 非法的定义或选项描述
   * @param cause - 底层定义期错误
   */
  constructor(message: string, cause?: unknown) {
    super(message, {
      code: 'finance_definition_error',
      ...(cause === undefined ? {} : { cause }),
    });
  }
}

/** 运行时值域、时序或估值错误。 */
export class FinanceValueError extends FinanceError {
  /**
   * @param message - 非法值或运行时状态描述
   * @param cause - 底层错误
   */
  constructor(message: string, cause?: unknown) {
    super(message, {
      code: 'finance_value_error',
      ...(cause === undefined ? {} : { cause }),
    });
  }
}

/** 两个货币定义不一致。 */
export class CurrencyMismatchError extends FinanceError {
  /** 期望的货币标识。 */
  readonly expectedCurrency: string;
  /** 实际传入的货币标识。 */
  readonly actualCurrency: string;

  /**
   * @param expectedCurrency - 期望货币（code/decimals）
   * @param actualCurrency - 实际货币（code/decimals）
   */
  constructor(expectedCurrency: string, actualCurrency: string) {
    super(`货币不一致（期望 ${expectedCurrency}，实际 ${actualCurrency}）`, {
      code: 'currency_mismatch',
    });
    this.expectedCurrency = expectedCurrency;
    this.actualCurrency = actualCurrency;
  }
}

/** 两个金融标的 key 不一致。 */
export class InstrumentMismatchError extends FinanceError {
  /** 期望的标的 key。 */
  readonly expectedInstrument: string;
  /** 实际传入的标的 key。 */
  readonly actualInstrument: string;

  /**
   * @param expectedInstrument - 期望标的 key
   * @param actualInstrument - 实际标的 key
   */
  constructor(expectedInstrument: string, actualInstrument: string) {
    super(`标的不一致（期望 ${expectedInstrument}，实际 ${actualInstrument}）`, {
      code: 'instrument_mismatch',
    });
    this.expectedInstrument = expectedInstrument;
    this.actualInstrument = actualInstrument;
  }
}

/** 行情快照、K 线或盘口非法。 */
export class MarketDataValidationError extends FinanceError {
  /**
   * @param message - 非法的行情结构描述
   * @param cause - 底层错误
   */
  constructor(message: string, cause?: unknown) {
    super(`行情数据非法: ${message}`, {
      code: 'market_data_validation_error',
      ...(cause === undefined ? {} : { cause }),
    });
  }
}

/** 订单状态机非法转移或订单时序错误。 */
export class OrderStateError extends FinanceError {
  /** 订单 id。 */
  readonly orderId: string;
  /** 转移前状态。 */
  readonly from: string;
  /** 触发事件。 */
  readonly event: string;

  /**
   * @param orderId - 订单 id
   * @param from - 转移前状态
   * @param event - 触发事件
   * @param detail - 补充原因
   */
  constructor(orderId: string, from: string, event: string, detail?: string) {
    super(
      `订单 "${orderId}" 在状态 ${from} 下不能处理事件 ${event}${detail === undefined ? '' : `: ${detail}`}`,
      { code: 'order_state_error' },
    );
    this.orderId = orderId;
    this.from = from;
    this.event = event;
  }
}

/** 成交数量超过订单剩余数量。 */
export class OrderOverfillError extends FinanceError {
  /** 订单 id。 */
  readonly orderId: string;
  /** 剩余数量（十进制字符串）。 */
  readonly remaining: string;
  /** 本次成交数量（十进制字符串）。 */
  readonly fillQuantity: string;

  /**
   * @param orderId - 订单 id
   * @param remaining - 剩余数量
   * @param fillQuantity - 本次成交数量
   */
  constructor(orderId: string, remaining: string, fillQuantity: string) {
    super(`订单 "${orderId}" 超量成交：剩余 ${remaining}，本次成交 ${fillQuantity}`, {
      code: 'order_overfill',
    });
    this.orderId = orderId;
    this.remaining = remaining;
    this.fillQuantity = fillQuantity;
  }
}

/** 汇率定义非法或冲突。 */
export class FxRateError extends FinanceError {
  /**
   * @param message - 汇率定义问题描述
   */
  constructor(message: string) {
    super(`汇率定义非法: ${message}`, { code: 'fx_rate_error' });
  }
}

/** 没有可用的 FX 换算路径。 */
export class MissingFxRateError extends FinanceError {
  /** 源货币 code。 */
  readonly fromCurrency: string;
  /** 目标货币 code。 */
  readonly toCurrency: string;

  /**
   * @param fromCurrency - 源货币 code
   * @param toCurrency - 目标货币 code
   */
  constructor(fromCurrency: string, toCurrency: string) {
    super(`缺少 ${fromCurrency} -> ${toCurrency} 的汇率路径`, { code: 'missing_fx_rate' });
    this.fromCurrency = fromCurrency;
    this.toCurrency = toCurrency;
  }
}

/** 组合估值缺少标的 mark price。 */
export class MissingMarkPriceError extends FinanceError {
  /** 缺少报价的标的 key。 */
  readonly instrumentKey: string;

  /**
   * @param instrumentKey - 标的 key
   */
  constructor(instrumentKey: string) {
    super(`组合估值缺少标的 "${instrumentKey}" 的 mark price`, {
      code: 'missing_mark_price',
    });
    this.instrumentKey = instrumentKey;
  }
}

/** 组合现金、结构或估值错误。 */
export class PortfolioError extends FinanceError {
  /**
   * @param message - 组合错误描述
   * @param options - 错误码与 cause
   */
  constructor(message: string, options?: KurumiErrorOptions) {
    super(message, { code: 'portfolio_error', ...options });
  }
}

export {
  DecimalDivisionByZeroError,
  DecimalOverflowError,
  DecimalParseError,
  DecimalRoundingError,
  MathDefinitionError,
  MathError,
  MathValueError,
  OptionModelError,
} from '@kurumi/math';
