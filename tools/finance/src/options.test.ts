/**
 * @file 工具上下文解析测试。
 * @description 覆盖标的注册表/自定义解析器、货币精度覆盖与错误。
 * @module finance-tools/options.test
 */

import { describe, expect, it } from 'vitest';

import { createCurrency, createInstrument, type Instrument } from '@kurumi/finance';
import { Decimal } from '@kurumi/math';

import { FinanceToolError } from './errors.js';
import { createToolContext } from './options.js';

const USDT = createCurrency('USDT', 8);
const btc = createInstrument({
  assetClass: 'crypto',
  venue: 'binance',
  symbol: 'BTC/USDT',
  baseAsset: 'BTC',
  quoteCurrency: USDT,
  tickSize: Decimal.parse('0.01'),
  lotSize: Decimal.parse('0.001'),
}) as Instrument;

describe('createToolContext', () => {
  it('通过注册表与货币覆盖解析', () => {
    const context = createToolContext({ instruments: [btc], currencies: { USDT: 6 } });
    expect(context.resolveInstrument('binance:BTC/USDT').symbol).toBe('BTC/USDT');
    expect(context.resolveCurrency('USDT').decimals).toBe(6);
    expect(context.resolveCurrency('USD').decimals).toBe(2);
  });

  it('优先使用自定义解析器', () => {
    const context = createToolContext({ resolveInstrument: () => btc });
    expect(context.resolveInstrument('anything').symbol).toBe('BTC/USDT');
  });

  it('未知标的与非法货币失败', () => {
    const context = createToolContext({ instruments: [btc] });
    expect(() => context.resolveInstrument('nope')).toThrow(FinanceToolError);
    expect(() => context.resolveCurrency('bad code')).toThrow(FinanceToolError);
  });
});
