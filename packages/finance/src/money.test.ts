/**
 * @file 货币与金额单元测试。
 * @description 覆盖 FC-MONEY-* 与币种不一致/序列化边界。
 * @module finance/money.test
 */

import { describe, expect, it } from 'vitest';

import { Decimal } from '@kurumi/math';
import {
  CurrencyMismatchError,
  DecimalDivisionByZeroError,
  FinanceDefinitionError,
} from './errors.js';
import {
  absMoney,
  addMoney,
  assertSameCurrency,
  compareMoney,
  createCurrency,
  createMoney,
  currencyEquals,
  currencyLabel,
  divideMoney,
  equalsMoney,
  formatMoney,
  isCurrency,
  isMoney,
  isZeroMoney,
  moneyFromJson,
  moneyToJson,
  multiplyMoney,
  negateMoney,
  parseMoney,
  rescaleMoney,
  subtractMoney,
  sumMoney,
  zeroMoney,
} from './money.js';
import { EUR, USD, USDT } from './fixtures.test-utils.js';

describe('createCurrency', () => {
  it('规范化代码并冻结', () => {
    const usd = createCurrency(' usd ', 2);
    expect(usd).toEqual({ code: 'USD', decimals: 2 });
    expect(Object.isFrozen(usd)).toBe(true);
    expect(currencyLabel(usd)).toBe('USD(2)');
  });

  it('拒绝非法代码与小数位', () => {
    expect(() => createCurrency('', 2)).toThrow(FinanceDefinitionError);
    expect(() => createCurrency('US', 2)).toThrow(FinanceDefinitionError);
    expect(() => createCurrency('1USD', 2)).toThrow(FinanceDefinitionError);
    expect(() => createCurrency('USD', -1)).toThrow(FinanceDefinitionError);
    expect(() => createCurrency('USD', 19)).toThrow(FinanceDefinitionError);
    expect(() => createCurrency('USD', 1.5)).toThrow(FinanceDefinitionError);
  });

  it('isCurrency / currencyEquals', () => {
    expect(isCurrency(USD)).toBe(true);
    expect(isCurrency({ code: 'USD', decimals: 2 })).toBe(true);
    expect(isCurrency({ code: 'usd', decimals: 2 })).toBe(false);
    expect(isCurrency(null)).toBe(false);
    expect(currencyEquals(USD, createCurrency('USD', 2))).toBe(true);
    expect(currencyEquals(USD, createCurrency('USD', 8))).toBe(false);
    expect(currencyEquals(USD, EUR)).toBe(false);
  });
});

describe('Money 构造与序列化', () => {
  it('createMoney/parseMoney/zeroMoney', () => {
    const money = createMoney(Decimal.parse('12.34'), USD);
    expect(money.amount.toString()).toBe('12.34');
    expect(isMoney(money)).toBe(true);
    expect(parseMoney('12.34', USD).amount.equals(money.amount)).toBe(true);
    expect(zeroMoney(USD).amount.isZero()).toBe(true);
  });

  it('拒绝非法金额/货币', () => {
    expect(() => createMoney('1' as never, USD)).toThrow(FinanceDefinitionError);
    expect(() => createMoney(Decimal.ONE, { code: 'usd', decimals: 2 } as never)).toThrow(
      FinanceDefinitionError,
    );
    expect(isMoney({ amount: Decimal.ONE })).toBe(false);
  });

  it('moneyToJson/moneyFromJson 往返', () => {
    const money = parseMoney('123.4500', USDT);
    const json = moneyToJson(money);
    expect(json).toEqual({ amount: '123.45', currency: 'USDT', decimals: 8 });
    expect(moneyFromJson(json).amount.toString()).toBe('123.45');
    expect(Object.isFrozen(json)).toBe(true);
  });

  it('moneyFromJson 拒绝非法结构', () => {
    expect(() => moneyFromJson(null)).toThrow(FinanceDefinitionError);
    expect(() => moneyFromJson({ amount: 1, currency: 'USD', decimals: 2 })).toThrow(
      FinanceDefinitionError,
    );
    expect(() => moneyFromJson({ amount: '1', currency: '', decimals: 2 })).toThrow(
      FinanceDefinitionError,
    );
    expect(() => moneyFromJson({ amount: '1', currency: 'USD', decimals: -1 })).toThrow(
      FinanceDefinitionError,
    );
  });
});

describe('Money 运算', () => {
  it('同币种加减、求和、比较', () => {
    const left = parseMoney('10.10', USD);
    const right = parseMoney('2.05', USD);
    expect(addMoney(left, right).amount.toString()).toBe('12.15');
    expect(subtractMoney(left, right).amount.toString()).toBe('8.05');
    expect(sumMoney([left, right], USD).amount.toString()).toBe('12.15');
    expect(sumMoney([], USD).amount.toString()).toBe('0');
    expect(compareMoney(left, right)).toBe(1);
    expect(compareMoney(left, left)).toBe(0);
    expect(equalsMoney(left, parseMoney('10.1', USD))).toBe(true);
    expect(isZeroMoney(zeroMoney(USD))).toBe(true);
  });

  it('跨币种运算显式失败', () => {
    const usd = parseMoney('10', USD);
    const eur = parseMoney('10', EUR);
    expect(() => addMoney(usd, eur)).toThrow(CurrencyMismatchError);
    expect(() => subtractMoney(usd, eur)).toThrow(CurrencyMismatchError);
    expect(() => compareMoney(usd, eur)).toThrow(CurrencyMismatchError);
    expect(() => sumMoney([usd, eur], USD)).toThrow(CurrencyMismatchError);
    expect(() => assertSameCurrency(usd, eur)).toThrow(CurrencyMismatchError);
    expect(equalsMoney(usd, eur)).toBe(false);
  });

  it('同 code 不同 decimals 视为不一致', () => {
    const usd2 = parseMoney('10', USD);
    const usd8 = createMoney(Decimal.parse('10'), createCurrency('USD', 8));
    expect(() => addMoney(usd2, usd8)).toThrow(CurrencyMismatchError);
  });

  it('乘除、取反与绝对值', () => {
    const money = parseMoney('10.00', USD);
    expect(multiplyMoney(money, Decimal.parse('1.5')).amount.toString()).toBe('15');
    expect(
      multiplyMoney(money, Decimal.parse('1.5'), { scale: 0, rounding: 'down' }).amount.toString(),
    ).toBe('15');
    expect(divideMoney(money, Decimal.parse('4')).amount.toString()).toBe('2.5');
    expect(() => divideMoney(money, Decimal.ZERO)).toThrow(DecimalDivisionByZeroError);
    expect(negateMoney(money).amount.toString()).toBe('-10');
    expect(absMoney(negateMoney(money)).amount.toString()).toBe('10');
  });

  it('rescaleMoney 与 formatMoney', () => {
    const money = parseMoney('123.456', USD);
    expect(rescaleMoney(money, 2).amount.toString()).toBe('123.46');
    expect(rescaleMoney(money, 2, 'down').amount.toString()).toBe('123.45');
    expect(formatMoney(money)).toBe('123.46 USD');
    expect(formatMoney(money, { decimals: 3, shouldIncludeCode: false })).toBe('123.456');
    expect(formatMoney(money, { decimals: 0, rounding: 'down' })).toBe('123 USD');
    expect(() => formatMoney(money, { decimals: -1 })).toThrow(FinanceDefinitionError);
    expect(() => formatMoney(money, { decimals: 31 })).toThrow(FinanceDefinitionError);
  });
});
