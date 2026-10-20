/**
 * @file 持仓属性测试：任意成交序列的净数量、重放与空仓不变量。
 * @description 覆盖 PBT-FIN-002 与 FC-POS-INV-001。
 * @module finance/position.properties.test
 */

import fc from 'fast-check';
import { describe, expect, it } from 'vitest';

import { Decimal } from '@kurumi/math';
import { createFill, OrderSide } from './order.js';
import { applyFillToPosition, createPosition, positionFromFills } from './position.js';
import { T0, btcUsdt } from './fixtures.test-utils.js';

const fillSpecArb = fc.record({
  isBuy: fc.boolean(),
  priceOffset: fc.integer({ min: 0, max: 2000 }),
  quantityUnits: fc.integer({ min: 1, max: 100 }),
});

describe('持仓属性', () => {
  it('净数量恒等于有符号成交量之和，重放结果一致', () => {
    fc.assert(
      fc.property(fc.array(fillSpecArb, { maxLength: 20 }), (specs) => {
        const instrument = btcUsdt();
        const fills = specs.map((spec, index) =>
          createFill({
            id: `f-${index}`,
            orderId: 'o-1',
            instrument,
            side: spec.isBuy ? OrderSide.Buy : OrderSide.Sell,
            price: Decimal.fromUnits(10_000n + BigInt(spec.priceOffset), 2),
            quantity: Decimal.fromUnits(BigInt(spec.quantityUnits), 1),
            timestampMs: T0 + index + 1,
          }),
        );

        let position = createPosition(instrument);
        let signedTotal = Decimal.ZERO;
        for (const fill of fills) {
          position = applyFillToPosition(position, fill).position;
          signedTotal = signedTotal.add(
            fill.side === OrderSide.Buy ? fill.quantity : fill.quantity.negate(),
          );
        }

        expect(position.netQuantity.equals(signedTotal)).toBe(true);
        if (signedTotal.isZero()) {
          expect(position.averageEntryPrice.isZero()).toBe(true);
        } else {
          expect(position.averageEntryPrice.isPositive()).toBe(true);
        }

        const replayed = positionFromFills(instrument, fills);
        expect(replayed.netQuantity.equals(position.netQuantity)).toBe(true);
        expect(replayed.averageEntryPrice.equals(position.averageEntryPrice)).toBe(true);
        expect(replayed.grossRealizedPnl.amount.equals(position.grossRealizedPnl.amount)).toBe(
          true,
        );
        expect(replayed.fees.amount.equals(position.fees.amount)).toBe(true);
      }),
      { numRuns: 150 },
    );
  });

  it('同价等量买卖回到空仓且已实现盈亏为零', () => {
    fc.assert(
      fc.property(
        fc.integer({ min: 1, max: 100 }),
        fc.integer({ min: 0, max: 2000 }),
        (quantityUnits, priceOffset) => {
          const instrument = btcUsdt();
          const price = Decimal.fromUnits(10_000n + BigInt(priceOffset), 2);
          const quantity = Decimal.fromUnits(BigInt(quantityUnits), 1);
          const opened = applyFillToPosition(
            createPosition(instrument),
            createFill({
              id: 'f-1',
              orderId: 'o-1',
              instrument,
              side: OrderSide.Buy,
              price,
              quantity,
              timestampMs: T0 + 1,
            }),
          ).position;
          const closed = applyFillToPosition(
            opened,
            createFill({
              id: 'f-2',
              orderId: 'o-1',
              instrument,
              side: OrderSide.Sell,
              price,
              quantity,
              timestampMs: T0 + 2,
            }),
          );
          expect(closed.position.netQuantity.isZero()).toBe(true);
          expect(closed.position.averageEntryPrice.isZero()).toBe(true);
          expect(closed.realizedPnlDelta.amount.isZero()).toBe(true);
        },
      ),
      { numRuns: 100 },
    );
  });
});
