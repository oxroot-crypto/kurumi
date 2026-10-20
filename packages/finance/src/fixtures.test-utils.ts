/**
 * @file 金融测试夹具：常用货币、标的、订单与成交。
 * @description 仅供单元测试使用，不进入构建产物与覆盖率统计。
 * @module finance/test-utils
 */

import { Decimal } from '@kurumi/math';
import {
  AssetClass,
  ExerciseStyle,
  OptionType,
  SettlementType,
  createInstrument,
  type FutureInstrument,
  type FxInstrument,
  type OptionInstrument,
  type PerpetualInstrument,
  type SpotInstrument,
} from './instrument.js';
import { createCurrency, createMoney } from './money.js';
import {
  OrderSide,
  OrderType,
  TimeInForce,
  createFill,
  createOrder,
  type CreateFillInput,
  type CreateOrderInput,
  type Fill,
  type Order,
} from './order.js';

/** 美元。 */
export const USD = createCurrency('USD', 2);
/** USDT。 */
export const USDT = createCurrency('USDT', 8);
/** 欧元。 */
export const EUR = createCurrency('EUR', 2);
/** 日元。 */
export const JPY = createCurrency('JPY', 0);
/** 比特币（作为基础资产/货币）。 */
export const BTC = createCurrency('BTC', 8);

/** 默认时间戳。 */
export const T0 = 1_700_000_000_000;

/**
 * 创建 BTC/USDT 现货标的。
 *
 * @returns 现货标的
 */
export function btcUsdt(): SpotInstrument {
  return createInstrument({
    assetClass: AssetClass.Crypto,
    venue: 'binance',
    symbol: 'BTC/USDT',
    baseAsset: 'BTC',
    quoteCurrency: USDT,
    tickSize: Decimal.parse('0.01'),
    lotSize: Decimal.parse('0.001'),
    minQuantity: Decimal.parse('0.001'),
    maxQuantity: Decimal.parse('1000'),
  }) as SpotInstrument;
}

/**
 * 创建 AAPL 股票标的。
 *
 * @returns 股票标的
 */
export function aapl(): SpotInstrument {
  return createInstrument({
    assetClass: AssetClass.Equity,
    venue: 'nasdaq',
    symbol: 'AAPL',
    baseAsset: 'AAPL',
    quoteCurrency: USD,
    tickSize: Decimal.parse('0.01'),
    lotSize: Decimal.ONE,
  }) as SpotInstrument;
}

/**
 * 创建 EUR/USD 外汇标的。
 *
 * @returns FX 标的
 */
export function eurusd(): FxInstrument {
  return createInstrument({
    assetClass: AssetClass.Fx,
    venue: 'fx',
    symbol: 'EUR/USD',
    baseCurrency: EUR,
    quoteCurrency: USD,
    tickSize: Decimal.parse('0.0001'),
    lotSize: Decimal.parse('1000'),
  }) as FxInstrument;
}

/**
 * 创建 ES 期货标的。
 *
 * @returns 期货标的
 */
export function esFuture(): FutureInstrument {
  return createInstrument({
    assetClass: AssetClass.Future,
    venue: 'cme',
    symbol: 'ES',
    quoteCurrency: USD,
    tickSize: Decimal.parse('0.25'),
    lotSize: Decimal.ONE,
    contractMultiplier: Decimal.parse('50'),
    expiryMs: T0 + 90 * 24 * 60 * 60_000,
    settlement: SettlementType.Cash,
  }) as FutureInstrument;
}

/**
 * 创建 BTC/USDT 永续标的。
 *
 * @returns 永续标的
 */
export function btcPerp(): PerpetualInstrument {
  return createInstrument({
    assetClass: AssetClass.Perpetual,
    venue: 'binance',
    symbol: 'BTCUSDT-PERP',
    quoteCurrency: USDT,
    tickSize: Decimal.parse('0.01'),
    lotSize: Decimal.parse('0.001'),
    contractMultiplier: Decimal.ONE,
    fundingIntervalMs: 8 * 60 * 60_000,
  }) as PerpetualInstrument;
}

/**
 * 创建 BTC/USDT 看涨期权。
 *
 * @returns 期权标的
 */
export function btcCall(): OptionInstrument {
  return createInstrument({
    assetClass: AssetClass.Option,
    venue: 'deribit',
    symbol: 'BTC-100000-C',
    quoteCurrency: USDT,
    tickSize: Decimal.parse('0.01'),
    lotSize: Decimal.parse('0.001'),
    contractMultiplier: Decimal.ONE,
    optionType: OptionType.Call,
    strike: Decimal.parse('100000'),
    expiryMs: T0 + 30 * 24 * 60 * 60_000,
    exerciseStyle: ExerciseStyle.European,
    settlement: SettlementType.Cash,
    underlying: 'BTC/USDT',
  }) as OptionInstrument;
}

/** 允许显式 undefined 的订单覆盖字段（便于测试字段矩阵）。 */
export type OrderOverrides = {
  readonly [K in keyof CreateOrderInput]?: CreateOrderInput[K] | undefined;
};

/** 默认订单输入。 */
export function orderInput(overrides: OrderOverrides = {}): CreateOrderInput {
  return {
    id: 'o-1',
    instrument: btcUsdt(),
    side: OrderSide.Buy,
    type: OrderType.Limit,
    timeInForce: TimeInForce.Gtc,
    quantity: Decimal.parse('1'),
    limitPrice: Decimal.parse('100'),
    createdAtMs: T0,
    ...overrides,
  } as CreateOrderInput;
}

/**
 * 创建默认订单（pending_new）。
 *
 * @param overrides - 覆盖字段
 * @returns 订单
 */
export function makeOrder(overrides: OrderOverrides = {}): Order {
  return createOrder(orderInput(overrides));
}

/**
 * 创建已接受订单（open）。
 *
 * @param overrides - 覆盖字段
 * @returns 订单
 */
export function openOrder(overrides: OrderOverrides = {}): Order {
  const order = makeOrder(overrides);
  return { ...order, status: 'open', updatedAtMs: order.createdAtMs + 1 };
}

/** 默认成交输入。 */
export function fillInput(overrides: Partial<CreateFillInput> = {}): CreateFillInput {
  return {
    id: 'f-1',
    orderId: 'o-1',
    instrument: btcUsdt(),
    side: OrderSide.Buy,
    price: Decimal.parse('100'),
    quantity: Decimal.parse('0.5'),
    timestampMs: T0 + 2,
    ...overrides,
  };
}

/**
 * 创建默认成交。
 *
 * @param overrides - 覆盖字段
 * @returns 成交
 */
export function makeFill(overrides: Partial<CreateFillInput> = {}): Fill {
  return createFill(fillInput(overrides));
}

/**
 * 创建 quote 货币手续费。
 *
 * @param amount - 金额字符串
 * @param currency - 默认 USDT
 * @returns 金额
 */
export function fee(amount: string, currency = USDT) {
  return createMoney(Decimal.parse(amount), currency);
}
