/**
 * @file 持仓与盈亏单元测试。
 * @description 覆盖开仓/加仓/减仓/平仓/反向穿透、乘数、手续费、盯市与错误边界。
 * @module finance/position.test
 */

import { describe, expect, it } from 'vitest';

import { Decimal } from '@kurumi/math';
import { CurrencyMismatchError, FinanceValueError, InstrumentMismatchError } from './errors.js';
import { createMoney } from './money.js';
import { OrderSide } from './order.js';
import {
  applyFillToPosition,
  createPosition,
  positionEntryNotional,
  positionFromFills,
  positionIsFlat,
  positionKey,
  positionMarkToMarket,
  positionNotional,
  positionReturn,
  positionTotalPnl,
  positionUnrealizedPnl,
} from './position.js';
import { EUR, T0, USD, btcUsdt, esFuture, makeFill } from './fixtures.test-utils.js';

function fill(side: 'buy' | 'sell', price: string, quantity: string, timestampMs = T0 + 2) {
  return makeFill({
    id: `f-${side}-${price}-${quantity}-${timestampMs}`,
    side,
    price: Decimal.parse(price),
    quantity: Decimal.parse(quantity),
    timestampMs,
  });
}

describe('createPosition', () => {
  it('创建空仓并冻结', () => {
    const position = createPosition(btcUsdt(), T0);
    expect(position.netQuantity.isZero()).toBe(true);
    expect(position.averageEntryPrice.isZero()).toBe(true);
    expect(position.grossRealizedPnl.amount.isZero()).toBe(true);
    expect(position.fees.amount.isZero()).toBe(true);
    expect(position.updatedAtMs).toBe(T0);
    expect(positionIsFlat(position)).toBe(true);
    expect(positionKey(position)).toBe('binance:BTC/USDT');
    expect(Object.isFrozen(position)).toBe(true);
  });
});

describe('applyFillToPosition', () => {
  it('开仓、加仓、部分减仓、平仓', () => {
    const empty = createPosition(btcUsdt());
    const opened = applyFillToPosition(empty, fill('buy', '100', '1'));
    expect(opened.position.netQuantity.toString()).toBe('1');
    expect(opened.position.averageEntryPrice.toString()).toBe('100');
    expect(opened.realizedPnlDelta.amount.isZero()).toBe(true);

    const added = applyFillToPosition(opened.position, fill('buy', '120', '1', T0 + 3));
    expect(added.position.netQuantity.toString()).toBe('2');
    expect(added.position.averageEntryPrice.toString()).toBe('110');

    const reduced = applyFillToPosition(added.position, fill('sell', '130', '1', T0 + 4));
    expect(reduced.position.netQuantity.toString()).toBe('1');
    expect(reduced.position.averageEntryPrice.toString()).toBe('110');
    expect(reduced.realizedPnlDelta.amount.toString()).toBe('20');
    expect(reduced.position.grossRealizedPnl.amount.toString()).toBe('20');

    const closed = applyFillToPosition(reduced.position, fill('sell', '90', '1', T0 + 5));
    expect(closed.position.netQuantity.isZero()).toBe(true);
    expect(closed.position.averageEntryPrice.isZero()).toBe(true);
    expect(closed.realizedPnlDelta.amount.toString()).toBe('-20');
    expect(closed.position.grossRealizedPnl.amount.isZero()).toBe(true);
    expect(positionIsFlat(closed.position)).toBe(true);
  });

  it('反向穿透只结算已平部分并按成交价建立反向仓位', () => {
    const opened = applyFillToPosition(createPosition(btcUsdt()), fill('buy', '100', '1')).position;
    const flipped = applyFillToPosition(opened, fill('sell', '120', '2', T0 + 3));
    expect(flipped.position.netQuantity.toString()).toBe('-1');
    expect(flipped.position.averageEntryPrice.toString()).toBe('120');
    expect(flipped.realizedPnlDelta.amount.toString()).toBe('20');
  });

  it('空头开仓与平仓盈亏方向正确', () => {
    const short = applyFillToPosition(createPosition(btcUsdt()), fill('sell', '100', '1')).position;
    expect(short.netQuantity.toString()).toBe('-1');
    expect(short.averageEntryPrice.toString()).toBe('100');

    const closed = applyFillToPosition(short, fill('buy', '80', '1', T0 + 3));
    expect(closed.position.netQuantity.isZero()).toBe(true);
    expect(closed.realizedPnlDelta.amount.toString()).toBe('20');
  });

  it('合约乘数参与已实现盈亏', () => {
    const future = esFuture();
    const opened = applyFillToPosition(
      createPosition(future),
      makeFill({
        instrument: future,
        price: Decimal.parse('5000'),
        quantity: Decimal.ONE,
        timestampMs: T0 + 2,
      }),
    ).position;
    const closed = applyFillToPosition(
      opened,
      makeFill({
        id: 'f-2',
        instrument: future,
        side: OrderSide.Sell,
        price: Decimal.parse('5100'),
        quantity: Decimal.ONE,
        timestampMs: T0 + 3,
      }),
    );
    expect(closed.realizedPnlDelta.amount.toString()).toBe('5000');
    expect(closed.realizedPnlDelta.currency).toEqual(USD);
  });

  it('累计手续费且总盈亏扣除手续费', () => {
    const opened = applyFillToPosition(
      createPosition(btcUsdt()),
      makeFill({
        quantity: Decimal.ONE,
        fee: createMoney(Decimal.parse('1'), btcUsdt().quoteCurrency),
        timestampMs: T0 + 2,
      }),
    );
    expect(opened.position.fees.amount.toString()).toBe('1');
    expect(opened.fee.amount.toString()).toBe('1');
    expect(positionTotalPnl(opened.position, Decimal.parse('110')).amount.toString()).toBe('9');
  });

  it('拒绝标的不一致、手续费币种不一致、时间倒退与非法价格/数量', () => {
    const position = createPosition(btcUsdt(), T0 + 10);
    expect(() => applyFillToPosition(position, fill('buy', '100', '1', T0 + 11))).not.toThrow();
    expect(() =>
      applyFillToPosition(position, makeFill({ instrument: esFuture(), timestampMs: T0 + 11 })),
    ).toThrow(InstrumentMismatchError);
    expect(() =>
      applyFillToPosition(
        position,
        makeFill({ fee: createMoney(Decimal.ONE, EUR), timestampMs: T0 + 11 }),
      ),
    ).toThrow(CurrencyMismatchError);
    expect(() => applyFillToPosition(position, fill('buy', '100', '1', T0 + 9))).toThrow(
      FinanceValueError,
    );
    const validFill = makeFill({ timestampMs: T0 + 11 });
    expect(() => applyFillToPosition(position, { ...validFill, price: Decimal.ZERO })).toThrow(
      FinanceValueError,
    );
    expect(() => applyFillToPosition(position, { ...validFill, quantity: Decimal.ZERO })).toThrow(
      FinanceValueError,
    );
  });
});

describe('持仓估值', () => {
  const long = applyFillToPosition(createPosition(btcUsdt()), fill('buy', '100', '2')).position;

  it('未实现盈亏、名义价值、开仓成本、总盈亏与收益率', () => {
    expect(positionUnrealizedPnl(long, Decimal.parse('110')).amount.toString()).toBe('20');
    expect(positionNotional(long, Decimal.parse('110')).amount.toString()).toBe('220');
    expect(positionEntryNotional(long).amount.toString()).toBe('200');
    expect(positionTotalPnl(long, Decimal.parse('110')).amount.toString()).toBe('20');
    expect(positionReturn(long, Decimal.parse('110'))?.toString()).toBe('0.1');
    expect(positionReturn(createPosition(btcUsdt()), Decimal.parse('100'))).toBeUndefined();
  });

  it('空头未实现盈亏与平仓零值', () => {
    const short = applyFillToPosition(createPosition(btcUsdt()), fill('sell', '100', '2')).position;
    expect(positionUnrealizedPnl(short, Decimal.parse('90')).amount.toString()).toBe('20');
    expect(positionUnrealizedPnl(short, Decimal.parse('110')).amount.toString()).toBe('-20');
    expect(
      positionUnrealizedPnl(createPosition(btcUsdt()), Decimal.parse('100')).amount.isZero(),
    ).toBe(true);
    expect(positionNotional(createPosition(btcUsdt()), Decimal.parse('100')).amount.isZero()).toBe(
      true,
    );
  });

  it('positionMarkToMarket 返回冻结快照', () => {
    const mark = positionMarkToMarket(long, Decimal.parse('110'));
    expect(mark.unrealizedPnl.amount.toString()).toBe('20');
    expect(mark.notional.amount.toString()).toBe('220');
    expect(mark.totalPnl.amount.toString()).toBe('20');
    expect(Object.isFrozen(mark)).toBe(true);
  });

  it('拒绝非正 mark price', () => {
    expect(() => positionUnrealizedPnl(long, Decimal.ZERO)).toThrow();
    expect(() => positionNotional(long, Decimal.ZERO)).toThrow();
  });
});

describe('positionFromFills', () => {
  it('按顺序重放与逐次应用一致', () => {
    const fills = [
      fill('buy', '100', '1', T0 + 2),
      fill('buy', '120', '1', T0 + 3),
      fill('sell', '130', '1', T0 + 4),
    ];
    let expected = createPosition(btcUsdt());
    for (const item of fills) expected = applyFillToPosition(expected, item).position;
    const replayed = positionFromFills(btcUsdt(), fills);
    expect(replayed.netQuantity.equals(expected.netQuantity)).toBe(true);
    expect(replayed.averageEntryPrice.equals(expected.averageEntryPrice)).toBe(true);
    expect(replayed.grossRealizedPnl.amount.equals(expected.grossRealizedPnl.amount)).toBe(true);
  });

  it('时间倒退失败', () => {
    const fills = [fill('buy', '100', '1', T0 + 3), fill('buy', '100', '1', T0 + 2)];
    expect(() => positionFromFills(btcUsdt(), fills)).toThrow(FinanceValueError);
  });
});
