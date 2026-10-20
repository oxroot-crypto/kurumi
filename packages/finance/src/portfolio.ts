/**
 * @file 多币种组合：现金、持仓、成交现金流、权益与敞口估值。
 * @description 现货/期权按带符号市值计入权益；期货/永续按未实现盈亏计入；跨币种必须提供 FX 转换器。
 * @module finance/portfolio
 */

import { Decimal } from '@kurumi/math';
import {
  CurrencyMismatchError,
  FinanceDefinitionError,
  FinanceValueError,
  MissingFxRateError,
  MissingMarkPriceError,
  PortfolioError,
} from './errors.js';
import { instrumentKey, isCashSettledDerivative, type Instrument } from './instrument.js';
import {
  createMoney,
  currencyEquals,
  currencyLabel,
  isCurrency,
  isMoney,
  negateMoney,
  subtractMoney,
  sumMoney,
  zeroMoney,
  type Currency,
  type Money,
} from './money.js';
import { OrderSide, type Fill } from './order.js';
import {
  applyFillToPosition,
  createPosition,
  positionEntryNotional,
  positionNotional,
  positionUnrealizedPnl,
  type Position,
} from './position.js';
import type { FxConverter } from './fx.js';
import { assertTimestampMs } from './validation.js';

/** 单币种现金余额。 */
export interface CashBalance {
  /** 货币。 */
  readonly currency: Currency;
  /** 余额，可为负（保证金账户）。 */
  readonly amount: Decimal;
}

/** 组合状态。 */
export interface Portfolio {
  /** 报表基准货币。 */
  readonly baseCurrency: Currency;
  /** 按货币 code 索引的现金余额。 */
  readonly cash: Readonly<Record<string, CashBalance>>;
  /** 按标的 key 索引的持仓。 */
  readonly positions: Readonly<Record<string, Position>>;
  /** 最后更新时间（epoch 毫秒）。 */
  readonly updatedAtMs: number;
}

/** 应用成交后的组合更新结果。 */
export interface PortfolioUpdate {
  /** 新组合。 */
  readonly portfolio: Portfolio;
  /** 更新后的持仓。 */
  readonly position: Position;
  /** 本次成交造成的 quote 货币现金变化（含手续费）。 */
  readonly cashDelta: Money;
}

/** 组合创建选项。 */
export interface CreatePortfolioOptions {
  /** 初始现金；同一货币不得重复。 */
  readonly cash?: readonly Money[];
  /** 初始时间，缺省 0。 */
  readonly timestampMs?: number;
}

/** 提现选项。 */
export interface WithdrawCashOptions {
  /** 是否允许余额为负，缺省 true（保证金账户）。 */
  readonly shouldAllowOverdraft?: boolean;
  /** 事件时间。 */
  readonly timestampMs?: number;
}

/** 组合估值输入。 */
export interface PortfolioValuation {
  /** 标的 key -> mark price。 */
  readonly marks: Readonly<Record<string, Decimal>>;
  /** 跨币种折算器；所有金额/标的不在 baseCurrency 时需要。 */
  readonly fx?: FxConverter;
}

/**
 * 创建组合。
 *
 * @param baseCurrency - 报表基准货币
 * @param options - 初始现金与时间
 * @returns 冻结组合
 * @throws {FinanceDefinitionError} 货币/现金/时间非法或现金货币重复
 *
 * @example
 * ```ts
 * const portfolio = createPortfolio(usd, {
 *   cash: [parseMoney('100000', usd)],
 * });
 * ```
 */
export function createPortfolio(
  baseCurrency: Currency,
  options?: CreatePortfolioOptions,
): Portfolio {
  if (!isCurrency(baseCurrency)) {
    throw new FinanceDefinitionError('portfolio.baseCurrency 非法');
  }
  const timestampMs = options?.timestampMs ?? 0;
  assertTimestampMs(timestampMs, 'portfolio.timestampMs');

  const cash: Record<string, CashBalance> = {};
  const seen = new Set<string>();
  for (const money of options?.cash ?? []) {
    if (!isMoney(money)) {
      throw new FinanceDefinitionError('portfolio.cash 包含非法金额');
    }
    if (seen.has(money.currency.code)) {
      throw new FinanceDefinitionError(`portfolio.cash 货币重复: ${money.currency.code}`);
    }
    seen.add(money.currency.code);
    cash[money.currency.code] = Object.freeze({
      currency: money.currency,
      amount: money.amount,
    });
  }

  return Object.freeze({
    baseCurrency,
    cash: Object.freeze(cash),
    positions: Object.freeze({}),
    updatedAtMs: timestampMs,
  });
}

/**
 * 查询某币种现金余额。
 *
 * @param portfolio - 组合
 * @param currency - 货币
 * @returns 金额；不存在时返回零
 * @throws {PortfolioError} 内部余额币种定义损坏
 */
export function portfolioCash(portfolio: Portfolio, currency: Currency): Money {
  if (!isCurrency(currency)) {
    throw new FinanceDefinitionError('currency 非法');
  }
  const balance = portfolio.cash[currency.code];
  if (balance === undefined) return zeroMoney(currency);
  if (!currencyEquals(balance.currency, currency)) {
    throw new PortfolioError(
      `组合余额币种与查询币种不一致（${currencyLabel(balance.currency)} vs ${currencyLabel(currency)}）`,
    );
  }
  return createMoney(balance.amount, balance.currency);
}

/**
 * 查询持仓。
 *
 * @param portfolio - 组合
 * @param instrument - 标的
 * @returns 持仓；不存在时 undefined
 */
export function portfolioPosition(
  portfolio: Portfolio,
  instrument: Instrument,
): Position | undefined {
  return portfolio.positions[instrumentKey(instrument)];
}

/**
 * 列出全部持仓。
 *
 * @param portfolio - 组合
 * @returns 冻结数组
 */
export function portfolioPositions(portfolio: Portfolio): readonly Position[] {
  return Object.freeze(Object.values(portfolio.positions));
}

/**
 * 查询某币种现金余额（按 code）。
 *
 * @param portfolio - 组合
 * @param code - 货币代码
 * @returns 金额；不存在时 undefined
 */
export function portfolioCashByCode(portfolio: Portfolio, code: string): Money | undefined {
  const balance = portfolio.cash[code];
  return balance === undefined ? undefined : createMoney(balance.amount, balance.currency);
}

/**
 * 增加/减少现金。
 *
 * @param portfolio - 组合
 * @param money - 带符号现金变化
 * @param timestampMs - 事件时间；缺省保持 updatedAt
 * @returns 新组合
 * @throws {FinanceValueError} 时间倒退
 */
export function applyCashDelta(
  portfolio: Portfolio,
  money: Money,
  timestampMs?: number,
): Portfolio {
  if (!isMoney(money)) {
    throw new FinanceDefinitionError('cash delta 非法');
  }
  const resolvedTimestamp = resolveTimestamp(portfolio, timestampMs);
  const key = money.currency.code;
  const existing = portfolio.cash[key];
  let nextAmount: Decimal;
  let currency = money.currency;
  if (existing === undefined) {
    nextAmount = money.amount;
  } else {
    if (!currencyEquals(existing.currency, money.currency)) {
      throw new CurrencyMismatchError(
        currencyLabel(existing.currency),
        currencyLabel(money.currency),
      );
    }
    currency = existing.currency;
    nextAmount = existing.amount.add(money.amount);
  }

  const nextCash: Record<string, CashBalance> = { ...portfolio.cash };
  nextCash[key] = Object.freeze({ currency, amount: nextAmount });
  return Object.freeze({
    ...portfolio,
    cash: Object.freeze(nextCash),
    updatedAtMs: resolvedTimestamp,
  });
}

/**
 * 存入现金。
 *
 * @param portfolio - 组合
 * @param money - 正金额
 * @param timestampMs - 事件时间
 * @returns 新组合
 * @throws {FinanceDefinitionError} 金额非正
 */
export function depositCash(portfolio: Portfolio, money: Money, timestampMs?: number): Portfolio {
  if (!money.amount.isPositive()) {
    throw new FinanceDefinitionError(
      `depositCash 金额必须大于 0（实际值: ${money.amount.toString()}）`,
    );
  }
  return applyCashDelta(portfolio, money, timestampMs);
}

/**
 * 提取现金。
 *
 * @param portfolio - 组合
 * @param money - 正金额
 * @param options - 是否允许透支与事件时间
 * @returns 新组合
 * @throws {FinanceDefinitionError} 金额非正
 * @throws {PortfolioError} 不允许透支且余额不足
 */
export function withdrawCash(
  portfolio: Portfolio,
  money: Money,
  options?: WithdrawCashOptions,
): Portfolio {
  if (!money.amount.isPositive()) {
    throw new FinanceDefinitionError(
      `withdrawCash 金额必须大于 0（实际值: ${money.amount.toString()}）`,
    );
  }
  if (options?.shouldAllowOverdraft === false) {
    const balance = portfolioCash(portfolio, money.currency);
    if (balance.amount.compare(money.amount) < 0) {
      throw new PortfolioError(
        `余额不足（${currencyLabel(money.currency)} 余额 ${balance.amount.toString()}，提现 ${money.amount.toString()}）`,
        { code: 'insufficient_cash' },
      );
    }
  }
  return applyCashDelta(portfolio, negateMoney(money), options?.timestampMs);
}

/**
 * 应用成交到组合。
 *
 * @param portfolio - 组合
 * @param fill - 成交
 * @returns 新组合、持仓与现金变化
 * @throws {FinanceValueError} 成交时间早于组合更新时间
 * @throws {InstrumentMismatchError} 标的不一致
 * @throws {CurrencyMismatchError} 手续费币种不是 quote
 */
export function applyFillToPortfolio(portfolio: Portfolio, fill: Fill): PortfolioUpdate {
  if (fill.timestampMs < portfolio.updatedAtMs) {
    throw new FinanceValueError(
      `fill.timestampMs ${fill.timestampMs} 早于组合更新时间 ${portfolio.updatedAtMs}`,
    );
  }

  const key = instrumentKey(fill.instrument);
  const existing = portfolio.positions[key];
  const basePosition = existing ?? createPosition(fill.instrument, fill.timestampMs);
  const update = applyFillToPosition(basePosition, fill);
  const fee = update.fee;

  let cashDelta: Money;
  if (isCashSettledDerivative(fill.instrument)) {
    cashDelta = subtractMoney(update.realizedPnlDelta, fee);
  } else {
    const signedQuantity = fill.side === OrderSide.Buy ? fill.quantity : fill.quantity.negate();
    const signedNotional = createMoney(
      fill.price.multiply(signedQuantity).multiply(fill.instrument.contractMultiplier),
      fill.instrument.quoteCurrency,
    );
    cashDelta = subtractMoney(negateMoney(signedNotional), fee);
  }

  const nextCashPortfolio = applyCashDelta(portfolio, cashDelta, fill.timestampMs);
  const nextPositions: Record<string, Position> = { ...nextCashPortfolio.positions };
  nextPositions[key] = update.position;

  return Object.freeze({
    portfolio: Object.freeze({
      ...nextCashPortfolio,
      positions: Object.freeze(nextPositions),
      updatedAtMs: fill.timestampMs,
    }),
    position: update.position,
    cashDelta,
  });
}

/**
 * 计算组合现金总价值（折算 base）。
 *
 * @param portfolio - 组合
 * @param fx - 可选 FX 转换器
 * @returns base 货币金额
 * @throws {MissingFxRateError} 需要换算但未提供转换器
 */
export function portfolioCashValue(portfolio: Portfolio, fx?: FxConverter): Money {
  const values = Object.values(portfolio.cash).map((balance) =>
    convertToBase(createMoney(balance.amount, balance.currency), portfolio.baseCurrency, fx),
  );
  return sumMoney(values, portfolio.baseCurrency);
}

/**
 * 计算组合权益。
 *
 * @param portfolio - 组合
 * @param valuation - marks 与可选 FX
 * @returns base 货币权益 = 现金 + 持仓价值
 * @throws {MissingMarkPriceError} 非空持仓缺少 mark
 * @throws {MissingFxRateError} 需要换算但未提供转换器
 */
export function portfolioEquity(portfolio: Portfolio, valuation: PortfolioValuation): Money {
  const cash = portfolioCashValue(portfolio, valuation.fx);
  const positionValues = portfolioPositions(portfolio).map((position) =>
    convertToBase(
      positionEquityValue(position, valuation.marks),
      portfolio.baseCurrency,
      valuation.fx,
    ),
  );
  return sumMoney([cash, ...positionValues], portfolio.baseCurrency);
}

/**
 * 计算组合毛敞口（各持仓名义价值绝对值之和，折算 base）。
 *
 * @param portfolio - 组合
 * @param valuation - marks 与可选 FX
 * @returns base 货币金额
 */
export function portfolioGrossExposure(portfolio: Portfolio, valuation: PortfolioValuation): Money {
  const values = portfolioPositions(portfolio).map((position) => {
    if (position.netQuantity.isZero()) return zeroMoney(portfolio.baseCurrency);
    const mark = requireMark(position, valuation.marks);
    return convertToBase(positionNotional(position, mark), portfolio.baseCurrency, valuation.fx);
  });
  return sumMoney(values, portfolio.baseCurrency);
}

/**
 * 计算组合净敞口（带符号名义价值之和，折算 base）。
 *
 * @param portfolio - 组合
 * @param valuation - marks 与可选 FX
 * @returns base 货币金额
 */
export function portfolioNetExposure(portfolio: Portfolio, valuation: PortfolioValuation): Money {
  const values = portfolioPositions(portfolio).map((position) => {
    if (position.netQuantity.isZero()) return zeroMoney(portfolio.baseCurrency);
    const mark = requireMark(position, valuation.marks);
    const signedNotional = createMoney(
      mark.multiply(position.netQuantity).multiply(position.instrument.contractMultiplier),
      position.instrument.quoteCurrency,
    );
    return convertToBase(signedNotional, portfolio.baseCurrency, valuation.fx);
  });
  return sumMoney(values, portfolio.baseCurrency);
}

/**
 * 计算组合未实现盈亏（折算 base）。
 *
 * @param portfolio - 组合
 * @param valuation - marks 与可选 FX
 * @returns base 货币金额
 */
export function portfolioUnrealizedPnl(portfolio: Portfolio, valuation: PortfolioValuation): Money {
  const values = portfolioPositions(portfolio).map((position) => {
    if (position.netQuantity.isZero()) return zeroMoney(portfolio.baseCurrency);
    const mark = requireMark(position, valuation.marks);
    return convertToBase(
      positionUnrealizedPnl(position, mark),
      portfolio.baseCurrency,
      valuation.fx,
    );
  });
  return sumMoney(values, portfolio.baseCurrency);
}

/**
 * 计算组合已实现盈亏净额（毛已实现 - 手续费，折算 base）。
 *
 * @param portfolio - 组合
 * @param fx - 可选 FX 转换器
 * @returns base 货币金额
 */
export function portfolioRealizedPnl(portfolio: Portfolio, fx?: FxConverter): Money {
  const values = portfolioPositions(portfolio).map((position) =>
    convertToBase(
      subtractMoney(position.grossRealizedPnl, position.fees),
      portfolio.baseCurrency,
      fx,
    ),
  );
  return sumMoney(values, portfolio.baseCurrency);
}

/**
 * 计算单持仓权益价值。
 *
 * @param position - 持仓
 * @param marks - mark price 映射
 * @returns quote 货币价值：期货/永续为未实现盈亏，其余为带符号市值
 */
export function positionEquityValue(
  position: Position,
  marks: Readonly<Record<string, Decimal>>,
): Money {
  if (position.netQuantity.isZero()) return zeroMoney(position.instrument.quoteCurrency);
  const mark = requireMark(position, marks);
  if (isCashSettledDerivative(position.instrument)) {
    return positionUnrealizedPnl(position, mark);
  }
  return createMoney(
    mark.multiply(position.netQuantity).multiply(position.instrument.contractMultiplier),
    position.instrument.quoteCurrency,
  );
}

/**
 * 计算单持仓开仓成本（quote）。
 *
 * @param position - 持仓
 * @returns 开仓成本名义价值
 */
export function positionCost(position: Position): Money {
  return positionEntryNotional(position);
}

/** 读取 mark price。 */
function requireMark(position: Position, marks: Readonly<Record<string, Decimal>>): Decimal {
  const key = instrumentKey(position.instrument);
  const mark = marks[key];
  if (mark === undefined) throw new MissingMarkPriceError(key);
  if (!mark.isPositive()) {
    throw new FinanceValueError(`mark price 必须大于 0（${key}: ${mark.toString()}）`);
  }
  return mark;
}

/** 折算到 base 货币。 */
function convertToBase(money: Money, baseCurrency: Currency, fx?: FxConverter): Money {
  if (currencyEquals(money.currency, baseCurrency)) return money;
  if (fx === undefined) {
    throw new MissingFxRateError(money.currency.code, baseCurrency.code);
  }
  return fx.convert(money, baseCurrency);
}

/** 解析组合事件时间。 */
function resolveTimestamp(portfolio: Portfolio, timestampMs: number | undefined): number {
  if (timestampMs === undefined) return portfolio.updatedAtMs;
  if (!Number.isSafeInteger(timestampMs) || timestampMs < 0) {
    throw new FinanceValueError(`timestampMs 必须是非负安全整数（实际值: ${String(timestampMs)}）`);
  }
  if (timestampMs < portfolio.updatedAtMs) {
    throw new FinanceValueError(
      `timestampMs ${timestampMs} 早于组合更新时间 ${portfolio.updatedAtMs}`,
    );
  }
  return timestampMs;
}
