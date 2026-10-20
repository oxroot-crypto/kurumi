/**
 * @file 公司行为单元测试：拆股、股票股利、现金分红与历史价格/数量调整。
 * @description 覆盖 FC-CA-*。
 * @module finance/corporate-actions.test
 */

import { describe, expect, it } from 'vitest';

import { Decimal } from '@kurumi/math';
import { FinanceDefinitionError } from './errors.js';
import {
  adjustHistoricPrices,
  adjustHistoricQuantities,
  adjustPriceForSplit,
  adjustQuantityForSplit,
  createCashDividend,
  createStockDividend,
  createStockSplit,
  isCorporateAction,
  totalReturnWithDividends,
} from './corporate-actions.js';
import { createMoney } from './money.js';
import { T0, USD } from './fixtures.test-utils.js';

const DAY_MS = 86_400_000;

describe('create* / isCorporateAction', () => {
  it('创建拆股、股票股利与现金分红', () => {
    const split = createStockSplit(Decimal.parse('2'), T0);
    const stockDividend = createStockDividend(Decimal.parse('0.1'), T0);
    const cashDividend = createCashDividend(createMoney(Decimal.parse('0.5'), USD), T0);
    expect(split.type).toBe('split');
    expect(stockDividend.type).toBe('stock_dividend');
    expect(cashDividend.type).toBe('cash_dividend');
    expect(isCorporateAction(split)).toBe(true);
    expect(isCorporateAction(stockDividend)).toBe(true);
    expect(isCorporateAction(cashDividend)).toBe(true);
    expect(Object.isFrozen(split)).toBe(true);
  });

  it('拒绝非法行为', () => {
    expect(() => createStockSplit(Decimal.ZERO, T0)).toThrow(FinanceDefinitionError);
    expect(() => createStockSplit(Decimal.ONE, -1)).toThrow(FinanceDefinitionError);
    expect(() => createStockDividend(Decimal.ZERO, T0)).toThrow(FinanceDefinitionError);
    expect(() => createCashDividend(createMoney(Decimal.parse('-1'), USD), T0)).toThrow(
      FinanceDefinitionError,
    );
    expect(() => createCashDividend({ amount: Decimal.ONE } as never, T0)).toThrow(
      FinanceDefinitionError,
    );
    expect(isCorporateAction({ type: 'split', ratio: Decimal.ONE, exDateMs: -1 })).toBe(false);
    expect(isCorporateAction({ type: 'unknown', exDateMs: T0 })).toBe(false);
  });
});

describe('价格与数量调整', () => {
  it('拆股价格/数量换算', () => {
    expect(adjustPriceForSplit(Decimal.parse('100'), Decimal.parse('2')).toString()).toBe('50');
    expect(adjustQuantityForSplit(Decimal.parse('100'), Decimal.parse('2')).toString()).toBe('200');
    expect(() => adjustPriceForSplit(Decimal.ZERO, Decimal.ONE)).toThrow(FinanceDefinitionError);
    expect(() => adjustQuantityForSplit(Decimal.parse('-1'), Decimal.ONE)).toThrow(
      FinanceDefinitionError,
    );
  });

  it('按除权日调整历史价格与数量，累计多次拆股', () => {
    const split = createStockSplit(Decimal.parse('2'), T0 + 2 * DAY_MS);
    const stockDividend = createStockDividend(Decimal.parse('0.1'), T0 + 3 * DAY_MS);
    const points = [
      { timestampMs: T0, price: Decimal.parse('100') },
      { timestampMs: T0 + 2 * DAY_MS, price: Decimal.parse('50') },
      { timestampMs: T0 + 3 * DAY_MS, price: Decimal.parse('45') },
      { timestampMs: T0 + 4 * DAY_MS, price: Decimal.parse('46') },
    ];
    const adjusted = adjustHistoricPrices(points, [split, stockDividend]);
    expect(adjusted[0]?.price.toString()).toBe('45.454545454545454545');
    expect(adjusted[1]?.price.toString()).toBe('45.454545454545454545');
    expect(adjusted[2]?.price.toString()).toBe('45');
    expect(adjusted[3]?.price.toString()).toBe('46');
    expect(points[0]?.price.toString()).toBe('100');

    const quantities = adjustHistoricQuantities(
      [
        { timestampMs: T0, quantity: Decimal.parse('100') },
        { timestampMs: T0 + 4 * DAY_MS, quantity: Decimal.parse('100') },
      ],
      [split, stockDividend],
    );
    expect(quantities[0]?.quantity.toString()).toBe('220');
    expect(quantities[1]?.quantity.toString()).toBe('100');
  });

  it('现金分红不改变历史价格', () => {
    const cashDividend = createCashDividend(createMoney(Decimal.parse('1'), USD), T0 + DAY_MS);
    const adjusted = adjustHistoricPrices(
      [{ timestampMs: T0, price: Decimal.parse('100') }],
      [cashDividend],
    );
    expect(adjusted[0]?.price.toString()).toBe('100');
  });

  it('拒绝非法价格/数量点与行为', () => {
    expect(() => adjustHistoricPrices([{ timestampMs: T0, price: Decimal.ZERO }], [])).toThrow(
      FinanceDefinitionError,
    );
    expect(() =>
      adjustHistoricQuantities([{ timestampMs: T0, quantity: Decimal.parse('-1') }], []),
    ).toThrow(FinanceDefinitionError);
    expect(() =>
      adjustHistoricPrices(
        [{ timestampMs: T0, price: Decimal.ONE }],
        [{ type: 'split', ratio: Decimal.ZERO, exDateMs: T0 }],
      ),
    ).toThrow(FinanceDefinitionError);
  });
});

describe('总回报', () => {
  it('含现金分红的总回报', () => {
    expect(
      totalReturnWithDividends(
        Decimal.parse('100'),
        Decimal.parse('110'),
        Decimal.parse('5'),
      ).toString(),
    ).toBe('0.15');
    expect(() => totalReturnWithDividends(Decimal.ZERO, Decimal.ONE, Decimal.ZERO)).toThrow(
      FinanceDefinitionError,
    );
    expect(() => totalReturnWithDividends(Decimal.ONE, Decimal.parse('-1'), Decimal.ZERO)).toThrow(
      FinanceDefinitionError,
    );
    expect(() => totalReturnWithDividends(Decimal.ONE, Decimal.ONE, Decimal.parse('-1'))).toThrow(
      FinanceDefinitionError,
    );
  });
});
