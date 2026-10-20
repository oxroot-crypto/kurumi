/**
 * @file 组合单元测试：现金、成交现金流、权益、敞口与多币种估值。
 * @description 覆盖 FC-PF-* 与现货/期货/期权三种会计路径。
 * @module finance/portfolio.test
 */

import { describe, expect, it } from 'vitest';

import { Decimal } from '@kurumi/math';
import {
  FinanceValueError,
  MissingFxRateError,
  MissingMarkPriceError,
  PortfolioError,
} from './errors.js';
import { createFxConverter, createFxRate } from './fx.js';
import { createMoney, parseMoney, zeroMoney } from './money.js';
import { OrderSide } from './order.js';
import {
  applyCashDelta,
  applyFillToPortfolio,
  createPortfolio,
  depositCash,
  portfolioCash,
  portfolioCashByCode,
  portfolioCashValue,
  portfolioEquity,
  portfolioGrossExposure,
  portfolioNetExposure,
  portfolioPosition,
  portfolioPositions,
  portfolioRealizedPnl,
  portfolioUnrealizedPnl,
  positionCost,
  positionEquityValue,
  withdrawCash,
} from './portfolio.js';
import { EUR, T0, USD, USDT, btcCall, btcUsdt, esFuture, makeFill } from './fixtures.test-utils.js';

function cashPortfolio(amount = '10000', currency = USDT) {
  return createPortfolio(currency, {
    cash: [parseMoney(amount, currency)],
    timestampMs: T0,
  });
}

function buyFill(amount: string, price: string, timestampMs = T0 + 2, fee?: string) {
  return makeFill({
    instrument: btcUsdt(),
    side: OrderSide.Buy,
    quantity: Decimal.parse(amount),
    price: Decimal.parse(price),
    timestampMs,
    ...(fee === undefined ? {} : { fee: createMoney(Decimal.parse(fee), USDT) }),
  });
}

describe('createPortfolio / 现金', () => {
  it('创建组合、查询现金与持仓', () => {
    const portfolio = cashPortfolio();
    expect(portfolio.baseCurrency).toEqual(USDT);
    expect(portfolioCash(portfolio, USDT).amount.toString()).toBe('10000');
    expect(portfolioCash(portfolio, USD).amount.isZero()).toBe(true);
    expect(portfolioCashByCode(portfolio, 'USDT')?.amount.toString()).toBe('10000');
    expect(portfolioCashByCode(portfolio, 'USD')).toBeUndefined();
    expect(portfolioPositions(portfolio)).toEqual([]);
    expect(Object.isFrozen(portfolio)).toBe(true);
    expect(Object.isFrozen(portfolio.cash)).toBe(true);
  });

  it('拒绝重复现金货币、非法金额与时间', () => {
    expect(() =>
      createPortfolio(USDT, {
        cash: [parseMoney('1', USDT), parseMoney('2', USDT)],
      }),
    ).toThrow();
    expect(() =>
      createPortfolio(USDT, {
        cash: [{ amount: Decimal.ONE } as never],
      }),
    ).toThrow();
    expect(() => createPortfolio(USDT, { timestampMs: -1 })).toThrow();
  });

  it('deposit/withdraw/overdraft/时间倒退', () => {
    const portfolio = cashPortfolio('100', USD);
    const deposited = depositCash(portfolio, parseMoney('50', USD), T0 + 1);
    expect(portfolioCash(deposited, USD).amount.toString()).toBe('150');

    const withdrawn = withdrawCash(deposited, parseMoney('40', USD), {
      timestampMs: T0 + 2,
      shouldAllowOverdraft: false,
    });
    expect(portfolioCash(withdrawn, USD).amount.toString()).toBe('110');

    expect(() =>
      withdrawCash(withdrawn, parseMoney('200', USD), {
        timestampMs: T0 + 3,
        shouldAllowOverdraft: false,
      }),
    ).toThrow(PortfolioError);

    const overdraft = withdrawCash(withdrawn, parseMoney('200', USD), {
      timestampMs: T0 + 3,
      shouldAllowOverdraft: true,
    });
    expect(portfolioCash(overdraft, USD).amount.toString()).toBe('-90');

    expect(() => depositCash(withdrawn, parseMoney('-1', USD), T0 + 3)).toThrow();
    expect(() => applyCashDelta(withdrawn, parseMoney('1', USD), T0 - 1)).toThrow(
      FinanceValueError,
    );
    const multiCurrency = applyCashDelta(withdrawn, parseMoney('1', EUR), T0 + 3);
    expect(portfolioCash(multiCurrency, EUR).amount.toString()).toBe('1');
  });
});

describe('现货成交现金流与权益', () => {
  it('买入并卖出后现金、手续费、已实现盈亏与权益守恒', () => {
    const portfolio = cashPortfolio('10000');
    const bought = applyFillToPortfolio(portfolio, buyFill('1', '100', T0 + 2, '1'));
    expect(bought.cashDelta.amount.toString()).toBe('-101');
    expect(portfolioCash(bought.portfolio, USDT).amount.toString()).toBe('9899');
    expect(bought.position.netQuantity.toString()).toBe('1');
    expect(portfolioPosition(bought.portfolio, btcUsdt())?.netQuantity.toString()).toBe('1');

    const mark = { [portfolioKey()]: Decimal.parse('100') };
    expect(portfolioEquity(bought.portfolio, { marks: mark }).amount.toString()).toBe('9999');
    expect(portfolioGrossExposure(bought.portfolio, { marks: mark }).amount.toString()).toBe('100');
    expect(portfolioNetExposure(bought.portfolio, { marks: mark }).amount.toString()).toBe('100');
    expect(portfolioUnrealizedPnl(bought.portfolio, { marks: mark }).amount.isZero()).toBe(true);

    const sold = applyFillToPortfolio(
      bought.portfolio,
      makeFill({
        id: 'f-2',
        instrument: btcUsdt(),
        side: OrderSide.Sell,
        quantity: Decimal.ONE,
        price: Decimal.parse('110'),
        timestampMs: T0 + 3,
        fee: createMoney(Decimal.ONE, USDT),
      }),
    );
    expect(sold.cashDelta.amount.toString()).toBe('109');
    expect(portfolioCash(sold.portfolio, USDT).amount.toString()).toBe('10008');
    expect(sold.position.netQuantity.isZero()).toBe(true);
    expect(sold.position.grossRealizedPnl.amount.toString()).toBe('10');
    expect(sold.position.fees.amount.toString()).toBe('2');
    expect(portfolioRealizedPnl(sold.portfolio).amount.toString()).toBe('8');
    expect(portfolioEquity(sold.portfolio, { marks: {} }).amount.toString()).toBe('10008');
  });

  it('平仓后不再需要 mark price', () => {
    const bought = applyFillToPortfolio(cashPortfolio(), buyFill('1', '100'));
    const sold = applyFillToPortfolio(
      bought.portfolio,
      makeFill({
        id: 'f-2',
        instrument: btcUsdt(),
        side: OrderSide.Sell,
        quantity: Decimal.ONE,
        price: Decimal.parse('100'),
        timestampMs: T0 + 3,
      }),
    );
    expect(portfolioEquity(sold.portfolio, { marks: {} }).amount.toString()).toBe('10000');
  });

  it('缺少 mark price 显式失败', () => {
    const bought = applyFillToPortfolio(cashPortfolio(), buyFill('1', '100'));
    expect(() => portfolioEquity(bought.portfolio, { marks: {} })).toThrow(MissingMarkPriceError);
    expect(() => portfolioGrossExposure(bought.portfolio, { marks: {} })).toThrow(
      MissingMarkPriceError,
    );
  });

  it('非正 mark 失败', () => {
    const bought = applyFillToPortfolio(cashPortfolio(), buyFill('1', '100'));
    expect(() =>
      portfolioEquity(bought.portfolio, {
        marks: { [portfolioKey()]: Decimal.ZERO },
      }),
    ).toThrow(FinanceValueError);
  });
});

describe('期货/期权会计', () => {
  it('期货开仓只扣手续费，平仓将已实现盈亏计入现金', () => {
    const portfolio = createPortfolio(USD, {
      cash: [parseMoney('10000', USD)],
      timestampMs: T0,
    });
    const future = esFuture();
    const opened = applyFillToPortfolio(
      portfolio,
      makeFill({
        instrument: future,
        side: OrderSide.Buy,
        price: Decimal.parse('5000'),
        quantity: Decimal.ONE,
        timestampMs: T0 + 2,
        fee: createMoney(Decimal.ONE, USD),
      }),
    );
    expect(opened.cashDelta.amount.toString()).toBe('-1');
    expect(portfolioCash(opened.portfolio, USD).amount.toString()).toBe('9999');
    expect(
      portfolioEquity(opened.portfolio, {
        marks: { [futureKey()]: Decimal.parse('5100') },
      }).amount.toString(),
    ).toBe('14999');

    const closed = applyFillToPortfolio(
      opened.portfolio,
      makeFill({
        id: 'f-2',
        instrument: future,
        side: OrderSide.Sell,
        price: Decimal.parse('5100'),
        quantity: Decimal.ONE,
        timestampMs: T0 + 3,
        fee: createMoney(Decimal.ONE, USD),
      }),
    );
    expect(closed.cashDelta.amount.toString()).toBe('4999');
    expect(portfolioCash(closed.portfolio, USD).amount.toString()).toBe('14998');
    expect(closed.position.netQuantity.isZero()).toBe(true);
    expect(portfolioEquity(closed.portfolio, { marks: {} }).amount.toString()).toBe('14998');
  });

  it('期权按溢价资产处理', () => {
    const portfolio = cashPortfolio('10000', USDT);
    const option = btcCall();
    const bought = applyFillToPortfolio(
      portfolio,
      makeFill({
        instrument: option,
        side: OrderSide.Buy,
        price: Decimal.parse('1000'),
        quantity: Decimal.ONE,
        timestampMs: T0 + 2,
        fee: createMoney(Decimal.ONE, USDT),
      }),
    );
    expect(bought.cashDelta.amount.toString()).toBe('-1001');
    expect(
      portfolioEquity(bought.portfolio, {
        marks: { [optionKey()]: Decimal.parse('1200') },
      }).amount.toString(),
    ).toBe('10199');
  });
});

describe('多币种估值', () => {
  it('现金与持仓跨币种折算', () => {
    const fx = createFxConverter([
      createFxRate({
        base: EUR,
        quote: USD,
        rate: Decimal.parse('1.1'),
        timestampMs: T0,
      }),
    ]);
    const portfolio = createPortfolio(USD, {
      cash: [createMoney(Decimal.parse('100'), EUR)],
      timestampMs: T0,
    });
    expect(() => portfolioCashValue(portfolio)).toThrow(MissingFxRateError);
    expect(portfolioCashValue(portfolio, fx).amount.toString()).toBe('110');
    expect(portfolioEquity(portfolio, { marks: {}, fx }).amount.toString()).toBe('110');
  });

  it('持仓 quote 与 base 不同需要 FX', () => {
    const portfolio = createPortfolio(USD, {
      cash: [parseMoney('1000', USD)],
      timestampMs: T0,
    });
    const bought = applyFillToPortfolio(portfolio, buyFill('1', '100'));
    const fx = createFxConverter([
      createFxRate({
        base: USDT,
        quote: USD,
        rate: Decimal.parse('1'),
        timestampMs: T0,
      }),
    ]);
    const marks = { [portfolioKey()]: Decimal.parse('110') };
    expect(portfolioEquity(bought.portfolio, { marks, fx }).amount.toString()).toBe('1010');
    expect(() => portfolioEquity(bought.portfolio, { marks })).toThrow(MissingFxRateError);
  });
});

describe('单持仓工具', () => {
  it('positionEquityValue 与 positionCost', () => {
    const bought = applyFillToPortfolio(cashPortfolio(), buyFill('1', '100'));
    expect(
      positionEquityValue(bought.position, {
        [portfolioKey()]: Decimal.parse('110'),
      }).amount.toString(),
    ).toBe('110');
    expect(positionCost(bought.position).amount.toString()).toBe('100');
    expect(zeroMoney(USDT).amount.isZero()).toBe(true);
  });
});

/** 默认现货标的 key。 */
function portfolioKey(): string {
  return 'binance:BTC/USDT';
}

/** 默认期货标的 key。 */
function futureKey(): string {
  return 'cme:ES';
}

/** 默认期权标的 key。 */
function optionKey(): string {
  return 'deribit:BTC-100000-C';
}
