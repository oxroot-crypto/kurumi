/**
 * @file 金融标的单元测试。
 * @description 覆盖标的判别联合、规格校验、注册表、tick/lot 对齐与名义价值。
 * @module finance/instrument.test
 */

import { describe, expect, it } from 'vitest';

import { Decimal } from '@kurumi/math';
import {
  CurrencyMismatchError,
  FinanceDefinitionError,
  InstrumentMismatchError,
} from './errors.js';
import {
  AssetClass,
  ExerciseStyle,
  OptionType,
  SettlementType,
  assertPriceValid,
  assertQuantityValid,
  assertSameInstrument,
  contractValue,
  createInstrument,
  createInstrumentRegistry,
  instrumentKey,
  isCashSettledDerivative,
  isInstrument,
  isPremiumInstrument,
  isPriceValid,
  isQuantityValid,
  notionalMoney,
  roundPrice,
  roundQuantity,
  sameInstrument,
  type FxInstrumentInput,
  type OptionInstrument,
  type OptionInstrumentInput,
  type SpotInstrument,
  type SpotInstrumentInput,
} from './instrument.js';
import { createMoney } from './money.js';
import {
  EUR,
  T0,
  USD,
  USDT,
  aapl,
  btcCall,
  btcPerp,
  btcUsdt,
  esFuture,
  eurusd,
} from './fixtures.test-utils.js';

function spotInput(overrides: Partial<SpotInstrumentInput> = {}): SpotInstrumentInput {
  return {
    assetClass: AssetClass.Crypto,
    venue: 'Binance',
    symbol: 'btc/usdt',
    baseAsset: 'btc',
    quoteCurrency: USDT,
    tickSize: Decimal.parse('0.01'),
    lotSize: Decimal.parse('0.001'),
    ...overrides,
  };
}

function fxInput(overrides: Partial<FxInstrumentInput> = {}): FxInstrumentInput {
  return {
    assetClass: AssetClass.Fx,
    venue: 'fx',
    symbol: 'eur/usd',
    baseCurrency: EUR,
    quoteCurrency: USD,
    tickSize: Decimal.parse('0.0001'),
    lotSize: Decimal.parse('1000'),
    ...overrides,
  };
}

function optionInput(overrides: Partial<OptionInstrumentInput> = {}): OptionInstrumentInput {
  return {
    assetClass: AssetClass.Option,
    venue: 'deribit',
    symbol: 'btc-100000-c',
    quoteCurrency: USDT,
    tickSize: Decimal.parse('0.01'),
    lotSize: Decimal.parse('0.001'),
    optionType: OptionType.Call,
    strike: Decimal.parse('100000'),
    expiryMs: T0 + 1_000_000,
    exerciseStyle: ExerciseStyle.European,
    settlement: SettlementType.Cash,
    underlying: 'btc/usdt',
    ...overrides,
  };
}

describe('createInstrument', () => {
  it('创建并规范化各类标的', () => {
    const spot = createInstrument(spotInput()) as SpotInstrument;
    expect(spot.assetClass).toBe('crypto');
    expect(spot.venue).toBe('binance');
    expect(spot.symbol).toBe('BTC/USDT');
    expect(spot.baseAsset).toBe('BTC');
    expect(spot.contractMultiplier.toString()).toBe('1');
    expect(spot.settlementCurrency).toEqual(USDT);
    expect(spot.isTradable).toBe(true);
    expect(Object.isFrozen(spot)).toBe(true);

    const fx = createInstrument(fxInput());
    expect(fx.assetClass).toBe('fx');
    expect(instrumentKey(fx)).toBe('fx:EUR/USD');

    const future = esFuture();
    expect(future.assetClass).toBe('future');
    expect(future.contractMultiplier.toString()).toBe('50');
    expect(future.settlement).toBe('cash');

    const perp = btcPerp();
    expect(perp.assetClass).toBe('perpetual');
    expect(perp.fundingIntervalMs).toBe(28_800_000);

    const option = createInstrument(optionInput()) as OptionInstrument;
    expect(option.assetClass).toBe('option');
    expect(option.underlying).toBe('BTC/USDT');
    expect(option.strike.toString()).toBe('100000');
  });

  it('支持 minQuantity/maxQuantity/minNotional 与 isTradable=false', () => {
    const instrument = createInstrument(
      spotInput({
        minQuantity: Decimal.parse('0.01'),
        maxQuantity: Decimal.parse('10'),
        minNotional: createMoney(Decimal.parse('10'), USDT),
        isTradable: false,
      }),
    );
    expect(instrument.minQuantity?.toString()).toBe('0.01');
    expect(instrument.maxQuantity?.toString()).toBe('10');
    expect(instrument.minNotional?.amount.toString()).toBe('10');
    expect(instrument.isTradable).toBe(false);
  });

  it('拒绝非法 tick/lot/multiplier/min/max/minNotional', () => {
    expect(() => createInstrument(spotInput({ tickSize: Decimal.ZERO }))).toThrow(
      FinanceDefinitionError,
    );
    expect(() => createInstrument(spotInput({ lotSize: Decimal.parse('-1') }))).toThrow(
      FinanceDefinitionError,
    );
    expect(() => createInstrument(spotInput({ contractMultiplier: Decimal.ZERO }))).toThrow(
      FinanceDefinitionError,
    );
    expect(() =>
      createInstrument(
        spotInput({ minQuantity: Decimal.parse('0.0011'), maxQuantity: Decimal.parse('1') }),
      ),
    ).toThrow(FinanceDefinitionError);
    expect(() =>
      createInstrument(spotInput({ minQuantity: Decimal.parse('2'), maxQuantity: Decimal.ONE })),
    ).toThrow(FinanceDefinitionError);
    expect(() =>
      createInstrument(spotInput({ minNotional: createMoney(Decimal.ONE, EUR) })),
    ).toThrow(CurrencyMismatchError);
    expect(() =>
      createInstrument(spotInput({ minNotional: createMoney(Decimal.ZERO, USDT) })),
    ).toThrow(FinanceDefinitionError);
  });

  it('拒绝非法 venue/symbol/baseAsset 与结算币种不一致', () => {
    expect(() => createInstrument(spotInput({ venue: 'BAD VENUE' }))).toThrow(
      FinanceDefinitionError,
    );
    expect(() => createInstrument(spotInput({ symbol: 'bad symbol' }))).toThrow(
      FinanceDefinitionError,
    );
    expect(() => createInstrument(spotInput({ baseAsset: 'BAD SPACE' }))).toThrow(
      FinanceDefinitionError,
    );
    expect(() => createInstrument(spotInput({ settlementCurrency: EUR }))).toThrow(
      CurrencyMismatchError,
    );
  });

  it('拒绝 FX 同币种、期货/期权时间与期权 strike 非法', () => {
    expect(() => createInstrument(fxInput({ baseCurrency: USD }))).toThrow(FinanceDefinitionError);
    expect(() => createInstrument(optionInput({ strike: Decimal.ZERO }))).toThrow(
      FinanceDefinitionError,
    );
    expect(() => createInstrument(optionInput({ expiryMs: 0 }))).toThrow(FinanceDefinitionError);
    expect(() => createInstrument(optionInput({ optionType: 'straddle' as never }))).toThrow(
      FinanceDefinitionError,
    );
    expect(() =>
      createInstrument({
        ...spotInput(),
        assetClass: 'bond',
      } as never),
    ).toThrow(FinanceDefinitionError);
  });
});

describe('isInstrument', () => {
  it('识别所有合法标的', () => {
    for (const instrument of [btcUsdt(), aapl(), eurusd(), esFuture(), btcPerp(), btcCall()]) {
      expect(isInstrument(instrument)).toBe(true);
    }
  });

  it('拒绝结构损坏的标的', () => {
    expect(isInstrument(null)).toBe(false);
    expect(isInstrument({})).toBe(false);
    expect(isInstrument({ ...btcUsdt(), assetClass: 'bond' })).toBe(false);
    expect(isInstrument({ ...btcUsdt(), tickSize: 1 })).toBe(false);
    expect(isInstrument({ ...btcUsdt(), settlementCurrency: EUR })).toBe(false);
    expect(isInstrument({ ...btcUsdt(), minQuantity: Decimal.ZERO })).toBe(false);
    expect(isInstrument({ ...btcUsdt(), maxQuantity: '10' })).toBe(false);
    expect(
      isInstrument({
        ...btcUsdt(),
        minQuantity: Decimal.parse('2'),
        maxQuantity: Decimal.ONE,
      }),
    ).toBe(false);
    expect(isInstrument({ ...btcUsdt(), minNotional: createMoney(Decimal.ONE, EUR) })).toBe(false);
    expect(isInstrument({ ...btcCall(), optionType: 'straddle' })).toBe(false);
    expect(isInstrument({ ...btcCall(), strike: Decimal.ZERO })).toBe(false);
  });
});

describe('价格与数量约束', () => {
  it('tick/lot 整数倍校验', () => {
    const instrument = btcUsdt();
    expect(isPriceValid(instrument, Decimal.parse('100.01'))).toBe(true);
    expect(isPriceValid(instrument, Decimal.parse('100.001'))).toBe(false);
    expect(isPriceValid(instrument, Decimal.ZERO)).toBe(false);
    expect(isQuantityValid(instrument, Decimal.parse('0.001'))).toBe(true);
    expect(isQuantityValid(instrument, Decimal.parse('0.0005'))).toBe(false);
    expect(isQuantityValid(instrument, Decimal.ZERO)).toBe(false);
    expect(() => assertPriceValid(instrument, Decimal.parse('100.001'))).toThrow(
      FinanceDefinitionError,
    );
    expect(() => assertQuantityValid(instrument, Decimal.parse('0.0005'))).toThrow(
      FinanceDefinitionError,
    );
  });

  it('min/max 数量约束', () => {
    const instrument = createInstrument(
      spotInput({ minQuantity: Decimal.parse('0.01'), maxQuantity: Decimal.parse('1') }),
    );
    expect(isQuantityValid(instrument, Decimal.parse('0.001'))).toBe(false);
    expect(isQuantityValid(instrument, Decimal.parse('0.01'))).toBe(true);
    expect(isQuantityValid(instrument, Decimal.parse('1'))).toBe(true);
    expect(isQuantityValid(instrument, Decimal.parse('1.001'))).toBe(false);
  });

  it('roundPrice/roundQuantity', () => {
    const instrument = btcUsdt();
    expect(roundPrice(instrument, Decimal.parse('100.123')).toString()).toBe('100.12');
    expect(roundPrice(instrument, Decimal.parse('100.125')).toString()).toBe('100.12');
    expect(roundPrice(instrument, Decimal.parse('100.125'), 'half-up').toString()).toBe('100.13');
    expect(roundQuantity(instrument, Decimal.parse('1.2349')).toString()).toBe('1.234');
    expect(roundQuantity(instrument, Decimal.parse('1.2349'), 'up').toString()).toBe('1.235');
  });
});

describe('名义价值与标的身份', () => {
  it('notionalMoney / contractValue', () => {
    const instrument = btcUsdt();
    const notional = notionalMoney(instrument, Decimal.parse('100'), Decimal.parse('2'));
    expect(notional.amount.toString()).toBe('200');
    expect(notional.currency).toEqual(USDT);
    expect(contractValue(instrument, Decimal.parse('100')).amount.toString()).toBe('100');
    expect(
      notionalMoney(esFuture(), Decimal.parse('5000'), Decimal.parse('2')).amount.toString(),
    ).toBe('500000');
    expect(() => notionalMoney(instrument, Decimal.parse('-1'), Decimal.ONE)).toThrow(
      FinanceDefinitionError,
    );
    expect(() => notionalMoney(instrument, Decimal.ONE, Decimal.parse('-1'))).toThrow(
      FinanceDefinitionError,
    );
  });

  it('sameInstrument/assertSameInstrument/key', () => {
    const left = btcUsdt();
    const right = btcUsdt();
    expect(sameInstrument(left, right)).toBe(true);
    expect(instrumentKey(left)).toBe('binance:BTC/USDT');
    expect(() => assertSameInstrument(left, aapl())).toThrow(InstrumentMismatchError);
  });

  it('衍生品分类', () => {
    expect(isCashSettledDerivative(esFuture())).toBe(true);
    expect(isCashSettledDerivative(btcPerp())).toBe(true);
    expect(isCashSettledDerivative(btcUsdt())).toBe(false);
    expect(isPremiumInstrument(btcCall())).toBe(true);
    expect(isPremiumInstrument(esFuture())).toBe(false);
  });
});

describe('InstrumentRegistry', () => {
  it('按 key 查找、require 与 list', () => {
    const registry = createInstrumentRegistry([btcUsdt(), aapl(), btcCall()]);
    expect(registry.get('binance:BTC/USDT')?.symbol).toBe('BTC/USDT');
    expect(registry.require('nasdaq:AAPL').symbol).toBe('AAPL');
    expect(registry.list()).toHaveLength(3);
    expect(Object.isFrozen(registry.list())).toBe(true);
    expect(registry.get('missing')).toBeUndefined();
    expect(() => registry.require('missing')).toThrow(FinanceDefinitionError);
  });

  it('拒绝重复 key 与非法标的', () => {
    expect(() => createInstrumentRegistry([btcUsdt(), btcUsdt()])).toThrow(FinanceDefinitionError);
    expect(() => createInstrumentRegistry([{ id: 'x' } as never])).toThrow(FinanceDefinitionError);
  });
});
