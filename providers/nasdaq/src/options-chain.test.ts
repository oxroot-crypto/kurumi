/**
 * @file Nasdaq 期权链提供方测试。
 * @description 用固定 option-chain JSON 覆盖到期日、call/put 报价与错误。
 * @module nasdaq/options-chain.test
 */

import { describe, expect, it } from 'vitest';

import {
  OptionType,
  ProviderDataError,
  createCurrency,
  createInstrument,
  type FetchLike,
  type Instrument,
} from '@kurumi/finance';
import { Decimal } from '@kurumi/math';

import { createNasdaqOptionsProvider } from './options-chain.js';

const T0 = Date.UTC(2026, 9, 19);
const USD = createCurrency('USD', 2);
const spy = createInstrument({
  assetClass: 'equity',
  venue: 'nasdaq',
  symbol: 'SPY',
  baseAsset: 'SPY',
  quoteCurrency: USD,
  tickSize: Decimal.parse('0.01'),
  lotSize: Decimal.ONE,
}) as Instrument;

const chainPayload = {
  data: {
    table: {
      rows: [
        {
          expirygroup: 'October 20, 2026',
          expiryDate: null,
          c_Last: null,
          c_Bid: null,
          c_Ask: null,
          c_Volume: null,
          c_Openinterest: null,
          strike: null,
          p_Last: null,
          p_Bid: null,
          p_Ask: null,
          p_Volume: null,
          p_Openinterest: null,
        },
        {
          expirygroup: '',
          expiryDate: 'Oct 20',
          c_Last: '217.15',
          c_Bid: '215.64',
          c_Ask: '218.10',
          c_Volume: '23',
          c_Openinterest: '4',
          strike: '550.00',
          p_Last: '0.02',
          p_Bid: '--',
          p_Ask: '0.01',
          p_Volume: '--',
          p_Openinterest: '126',
        },
        {
          expirygroup: '',
          expiryDate: 'Oct 20',
          c_Last: '212.20',
          c_Bid: '210.62',
          c_Ask: '213.09',
          c_Volume: '9',
          c_Openinterest: '2',
          strike: '555.00',
          p_Last: '0.03',
          p_Bid: '0.01',
          p_Ask: '0.02',
          p_Volume: '5',
          p_Openinterest: '335',
        },
      ],
    },
  },
};

function jsonFetch(payload: unknown): FetchLike {
  return async () =>
    new Response(JSON.stringify(payload), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    });
}

describe('createNasdaqOptionsProvider', () => {
  it('解析到期日与 call/put 合约', async () => {
    const provider = createNasdaqOptionsProvider({
      assetClass: 'etf',
      fetchImpl: jsonFetch(chainPayload),
      nowMs: () => T0,
    });
    const expirations = await provider.getExpirations(spy);
    expect(expirations).toEqual([Date.UTC(2026, 9, 20)]);

    const chain = await provider.getChain(spy, Date.UTC(2026, 9, 20));
    expect(chain.contracts).toHaveLength(4);
    const call = chain.contracts.find(
      (item) => item.optionType === OptionType.Call && item.strike.toString() === '550',
    );
    expect(call?.bid?.toString()).toBe('215.64');
    expect(call?.ask?.toString()).toBe('218.1');
    expect(call?.volume?.toString()).toBe('23');
    expect(call?.openInterest?.toString()).toBe('4');

    const put = chain.contracts.find(
      (item) => item.optionType === OptionType.Put && item.strike.toString() === '550',
    );
    expect(put?.bid).toBeUndefined();
    expect(put?.ask?.toString()).toBe('0.01');
    expect(put?.last?.toString()).toBe('0.02');
    expect(put?.openInterest?.toString()).toBe('126');
  });

  it('未知到期日失败', async () => {
    const provider = createNasdaqOptionsProvider({
      fetchImpl: jsonFetch(chainPayload),
      nowMs: () => T0,
    });
    await expect(provider.getChain(spy, Date.UTC(2027, 0, 1))).rejects.toBeInstanceOf(
      ProviderDataError,
    );
  });

  it('到期日存在但没有任何合约时返回空链', async () => {
    const provider = createNasdaqOptionsProvider({
      fetchImpl: jsonFetch({
        data: {
          table: {
            rows: [
              { expirygroup: 'October 20, 2026', strike: null },
              { expirygroup: '', expiryDate: 'Oct 20', strike: '550.00' },
            ],
          },
        },
      }),
      nowMs: () => T0,
    });
    const chain = await provider.getChain(spy, Date.UTC(2026, 9, 20));
    expect(chain.contracts).toEqual([]);
  });

  it('非法 expirygroup 失败', async () => {
    const provider = createNasdaqOptionsProvider({
      fetchImpl: jsonFetch({
        data: { table: { rows: [{ expirygroup: 'not-a-date' }] } },
      }),
      nowMs: () => T0,
    });
    await expect(provider.getExpirations(spy)).rejects.toBeInstanceOf(ProviderDataError);
  });

  it('缺少 expirygroup 时返回空到期日列表', async () => {
    const provider = createNasdaqOptionsProvider({
      fetchImpl: jsonFetch({ data: { table: { rows: [] } } }),
      nowMs: () => T0,
    });
    await expect(provider.getExpirations(spy)).resolves.toEqual([]);
  });

  it('拒绝非法 expirationMs', async () => {
    const provider = createNasdaqOptionsProvider({
      fetchImpl: jsonFetch(chainPayload),
      nowMs: () => T0,
    });
    await expect(provider.getChain(spy, -1)).rejects.toBeInstanceOf(ProviderDataError);
  });
});
