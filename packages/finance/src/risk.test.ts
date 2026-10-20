/**
 * @file 风控单元测试：下单限额、组合风险、保证金与强平价格。
 * @description 覆盖 FC-RISK-* 与违规列表完整性。
 * @module finance/risk.test
 */

import { describe, expect, it } from 'vitest';

import { Decimal } from '@kurumi/math';
import { MissingMarkPriceError } from './errors.js';
import { createMoney, parseMoney } from './money.js';
import { OrderSide, acceptOrder } from './order.js';
import { applyFillToPosition, createPosition } from './position.js';
import { applyFillToPortfolio, createPortfolio } from './portfolio.js';
import {
  RiskViolationCode,
  calculateMarginRequirement,
  checkOrderRisk,
  checkOrderRiskForOrder,
  checkPortfolioRisk,
  liquidationPrice,
} from './risk.js';
import {
  T0,
  USD,
  USDT,
  aapl,
  btcUsdt,
  esFuture,
  makeFill,
  makeOrder,
} from './fixtures.test-utils.js';

const instrument = btcUsdt();

function openLong(quantity = '1') {
  return applyFillToPosition(
    createPosition(instrument),
    makeFill({
      price: Decimal.parse('100'),
      quantity: Decimal.parse(quantity),
      timestampMs: T0 + 1,
    }),
  ).position;
}

describe('checkOrderRisk', () => {
  it('合法订单无违规', () => {
    expect(
      checkOrderRisk({
        instrument,
        side: OrderSide.Buy,
        quantity: Decimal.ONE,
        price: Decimal.parse('100'),
        limits: {},
      }),
    ).toEqual([]);
  });

  it('标的不交易/不在允许集合', () => {
    const nonTradable = Object.freeze({ ...instrument, isTradable: false });
    const result = checkOrderRisk({
      instrument: nonTradable,
      side: OrderSide.Buy,
      quantity: Decimal.ONE,
      price: Decimal.parse('100'),
      limits: { allowedInstrumentKeys: ['nasdaq:AAPL'] },
    });
    expect(result.map((item) => item.code)).toEqual([
      RiskViolationCode.InstrumentNotTradable,
      RiskViolationCode.InstrumentNotAllowed,
    ]);
  });

  it('数量/价格非法时提前返回', () => {
    const result = checkOrderRisk({
      instrument,
      side: OrderSide.Buy,
      quantity: Decimal.parse('0.0005'),
      price: Decimal.parse('100.001'),
      limits: { maxOrderNotional: createMoney(Decimal.ONE, USDT) },
    });
    expect(result.map((item) => item.code)).toEqual([
      RiskViolationCode.InvalidQuantity,
      RiskViolationCode.InvalidPrice,
    ]);
  });

  it('订单名义价值超限与限额币种不一致', () => {
    const exceeded = checkOrderRisk({
      instrument,
      side: OrderSide.Buy,
      quantity: Decimal.ONE,
      price: Decimal.parse('100'),
      limits: { maxOrderNotional: createMoney(Decimal.parse('50'), USDT) },
    });
    expect(exceeded[0]?.code).toBe(RiskViolationCode.OrderNotionalExceeded);

    const mismatch = checkOrderRisk({
      instrument,
      side: OrderSide.Buy,
      quantity: Decimal.ONE,
      price: Decimal.parse('100'),
      limits: { maxOrderNotional: createMoney(Decimal.parse('200'), USD) },
    });
    expect(mismatch[0]?.code).toBe(RiskViolationCode.LimitCurrencyMismatch);
  });

  it('持仓数量/名义价值/标的不一致', () => {
    const position = openLong('1');
    const quantity = checkOrderRisk({
      instrument,
      side: OrderSide.Buy,
      quantity: Decimal.ONE,
      price: Decimal.parse('100'),
      position,
      limits: { maxPositionQuantity: Decimal.parse('1.5') },
    });
    expect(quantity[0]?.code).toBe(RiskViolationCode.PositionQuantityExceeded);

    const notional = checkOrderRisk({
      instrument,
      side: OrderSide.Buy,
      quantity: Decimal.ONE,
      price: Decimal.parse('100'),
      position,
      limits: { maxPositionNotional: createMoney(Decimal.parse('150'), USDT) },
    });
    expect(notional[0]?.code).toBe(RiskViolationCode.PositionNotionalExceeded);

    const mismatch = checkOrderRisk({
      instrument,
      side: OrderSide.Buy,
      quantity: Decimal.ONE,
      price: Decimal.parse('100'),
      position: applyFillToPosition(
        createPosition(aapl()),
        makeFill({
          instrument: aapl(),
          price: Decimal.parse('100'),
          quantity: Decimal.ONE,
          timestampMs: T0 + 1,
        }),
      ).position,
      limits: {},
    });
    expect(mismatch[0]?.code).toBe(RiskViolationCode.PositionInstrumentMismatch);
  });

  it('checkOrderRiskForOrder 使用剩余数量', () => {
    const order = acceptOrder(makeOrder({ quantity: Decimal.parse('2') }), T0 + 1);
    const result = checkOrderRiskForOrder(order, Decimal.parse('100'), {
      maxOrderNotional: createMoney(Decimal.parse('150'), USDT),
    });
    expect(result[0]?.code).toBe(RiskViolationCode.OrderNotionalExceeded);
  });
});

describe('checkPortfolioRisk', () => {
  function portfolioWithPosition() {
    const portfolio = createPortfolioWithCash();
    return applyFillToPortfolio(portfolio, makeFill({ quantity: Decimal.ONE, timestampMs: T0 + 2 }))
      .portfolio;
  }

  it('毛敞口/杠杆/最小权益/币种不一致', () => {
    const portfolio = portfolioWithPosition();
    const valuation = { marks: { 'binance:BTC/USDT': Decimal.parse('100') } };
    const result = checkPortfolioRisk({
      portfolio,
      valuation,
      limits: {
        maxGrossExposure: createMoney(Decimal.parse('50'), USDT),
        maxLeverage: Decimal.parse('0.001'),
        minEquity: createMoney(Decimal.parse('20000'), USDT),
      },
    });
    expect(result.map((item) => item.code)).toEqual([
      RiskViolationCode.GrossExposureExceeded,
      RiskViolationCode.LeverageExceeded,
      RiskViolationCode.MinEquityBreached,
    ]);

    const mismatch = checkPortfolioRisk({
      portfolio,
      valuation,
      limits: { maxGrossExposure: createMoney(Decimal.ONE, USD) },
    });
    expect(mismatch[0]?.code).toBe(RiskViolationCode.LimitCurrencyMismatch);
  });

  it('非正权益触发杠杆违规', () => {
    const portfolio = createPortfolioWithCash('0');
    const result = checkPortfolioRisk({
      portfolio,
      valuation: { marks: {} },
      limits: { maxLeverage: Decimal.ONE },
    });
    expect(result[0]?.code).toBe(RiskViolationCode.LeverageExceeded);
  });

  it('非法 maxLeverage 与缺少 mark', () => {
    const portfolio = createPortfolioWithCash();
    const invalid = checkPortfolioRisk({
      portfolio,
      valuation: { marks: {} },
      limits: { maxLeverage: Decimal.ZERO },
    });
    expect(invalid[0]?.code).toBe(RiskViolationCode.LeverageExceeded);

    const withPosition = portfolioWithPosition();
    expect(() =>
      checkPortfolioRisk({
        portfolio: withPosition,
        valuation: { marks: {} },
        limits: {},
      }),
    ).toThrow(MissingMarkPriceError);
  });
});

describe('保证金', () => {
  it('按名义价值与费率计算', () => {
    const result = calculateMarginRequirement(
      esFuture(),
      Decimal.parse('2'),
      Decimal.parse('5000'),
      { initial: Decimal.parse('0.1'), maintenance: Decimal.parse('0.05') },
    );
    expect(result.notional.amount.toString()).toBe('500000');
    expect(result.initialMargin.amount.toString()).toBe('50000');
    expect(result.maintenanceMargin.amount.toString()).toBe('25000');
    expect(result.notional.currency).toEqual(USD);
  });

  it('拒绝非法费率与负数量', () => {
    expect(() =>
      calculateMarginRequirement(esFuture(), Decimal.ONE, Decimal.ONE, {
        initial: Decimal.parse('1.1'),
        maintenance: Decimal.parse('0.5'),
      }),
    ).toThrow();
    expect(() =>
      calculateMarginRequirement(esFuture(), Decimal.ONE, Decimal.ONE, {
        initial: Decimal.parse('0.1'),
        maintenance: Decimal.parse('0.2'),
      }),
    ).toThrow();
    expect(() =>
      calculateMarginRequirement(esFuture(), Decimal.parse('-1'), Decimal.ONE, {
        initial: Decimal.parse('0.1'),
        maintenance: Decimal.parse('0.05'),
      }),
    ).toThrow();
  });
});

describe('强平价格', () => {
  it('多头与空头公式', () => {
    const long = openLong('1');
    expect(
      liquidationPrice(long, Decimal.parse('10'), Decimal.parse('0.005')).amount.toString(),
    ).toBe('90.5');

    const short = applyFillToPosition(
      createPosition(instrument),
      makeFill({
        side: OrderSide.Sell,
        price: Decimal.parse('100'),
        quantity: Decimal.ONE,
        timestampMs: T0 + 1,
      }),
    ).position;
    expect(
      liquidationPrice(short, Decimal.parse('10'), Decimal.parse('0.005')).amount.toString(),
    ).toBe('109.5');
  });

  it('拒绝空仓、非法杠杆与费率', () => {
    const empty = createPosition(instrument);
    expect(() => liquidationPrice(empty, Decimal.ONE, Decimal.ZERO)).toThrow();
    const long = openLong('1');
    expect(() => liquidationPrice(long, Decimal.ZERO, Decimal.ZERO)).toThrow();
    expect(() => liquidationPrice(long, Decimal.ONE, Decimal.parse('-0.1'))).toThrow();
    expect(() => liquidationPrice(long, Decimal.ONE, Decimal.ONE)).toThrow();
  });
});

/** 创建现金组合。 */
function createPortfolioWithCash(amount = '10000') {
  return createPortfolio(USDT, {
    cash: [parseMoney(amount, USDT)],
    timestampMs: T0,
  });
}
