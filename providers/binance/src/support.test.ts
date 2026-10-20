/**
 * @file Binance 共享解析与选项测试。
 * @description 覆盖 symbol 映射、货币、数值/整数解析、K 线校验与选项边界。
 * @module binance/support.test
 */

import { describe, expect, it } from 'vitest';

import {
  AssetClass,
  ProviderDataError,
  createCurrency,
  createInstrument,
  type Instrument,
} from '@kurumi/finance';
import { Decimal } from '@kurumi/math';

import {
  buildBarFromKline,
  currencyFor,
  optionalDecimal,
  requireArray,
  requireDecimal,
  requireInteger,
  requireRecord,
  resolveBinanceOptions,
  toBinanceSymbol,
} from './support.js';
import { BarInterval } from '@kurumi/finance';

const USDT = createCurrency('USDT', 8);
const btc = createInstrument({
  assetClass: AssetClass.Crypto,
  venue: 'binance',
  symbol: 'BTC/USDT',
  baseAsset: 'BTC',
  quoteCurrency: USDT,
  tickSize: Decimal.parse('0.01'),
  lotSize: Decimal.parse('0.001'),
}) as Instrument;
const perp = createInstrument({
  assetClass: AssetClass.Perpetual,
  venue: 'binance-futures',
  symbol: 'BTCUSDT',
  quoteCurrency: USDT,
  tickSize: Decimal.parse('0.1'),
  lotSize: Decimal.parse('0.001'),
  fundingIntervalMs: 28_800_000,
}) as Instrument;
const quarter = createInstrument({
  assetClass: AssetClass.Future,
  venue: 'binance-futures',
  symbol: 'BTCUSDT_260116',
  quoteCurrency: USDT,
  tickSize: Decimal.parse('0.1'),
  lotSize: Decimal.parse('0.001'),
  expiryMs: Date.UTC(2026, 0, 16),
  settlement: 'cash',
}) as Instrument;

describe('Binance support', () => {
  it('symbol 映射与货币', () => {
    expect(toBinanceSymbol(btc)).toBe('BTCUSDT');
    expect(toBinanceSymbol(perp)).toBe('BTCUSDT');
    expect(toBinanceSymbol(quarter)).toBe('BTCUSDT_260116');
    const settings = resolveBinanceOptions();
    expect(currencyFor(settings, 'USDT').decimals).toBe(8);
    expect(currencyFor(settings, 'JPY').decimals).toBe(0);
    expect(() => currencyFor(settings, 'BAD CODE')).toThrow(ProviderDataError);
  });

  it('数值/整数/对象解析', () => {
    expect(requireDecimal('1.5', 'v', 'test').toString()).toBe('1.5');
    expect(optionalDecimal('', 'v', 'test')).toBeUndefined();
    expect(() => requireDecimal(undefined, 'v', 'test')).toThrow(ProviderDataError);
    expect(() => optionalDecimal({}, 'v', 'test')).toThrow(ProviderDataError);
    expect(requireInteger('3', 'n', 'test')).toBe(3);
    expect(() => requireInteger(-1, 'n', 'test')).toThrow(ProviderDataError);
    expect(() => requireRecord([], '$', 'test')).toThrow(ProviderDataError);
    expect(() => requireArray({}, '$', 'test')).toThrow(ProviderDataError);
  });

  it('K 线转换与数组长度校验', () => {
    const settings = resolveBinanceOptions();
    const bar = buildBarFromKline(settings, btc, BarInterval.Minute1, [
      Date.UTC(2026, 9, 19),
      '100',
      '110',
      '90',
      '105',
      '10',
      '0',
      '0',
      3,
    ]);
    expect(bar.close.toString()).toBe('105');
    expect(bar.tradeCount).toBe(3);
    expect(() => buildBarFromKline(settings, btc, BarInterval.Minute1, [1, 2])).toThrow(
      ProviderDataError,
    );
  });

  it('选项校验', () => {
    expect(resolveBinanceOptions().baseUrl).toBe('https://api.binance.com');
    expect(() => resolveBinanceOptions({ id: ' ' })).toThrow(ProviderDataError);
    expect(() => resolveBinanceOptions({ baseUrl: 'not a url' })).toThrow(ProviderDataError);
    expect(() => resolveBinanceOptions({ futuresBaseUrl: 'not a url' })).toThrow(ProviderDataError);
  });
});
