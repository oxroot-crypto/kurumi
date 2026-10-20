/**
 * @file 美国财政部收益率曲线提供方测试。
 * @description 用固定 CSV 覆盖最新曲线、指定日期、USD 限制与错误。
 * @module us-treasury/provider.test
 */

import { describe, expect, it } from 'vitest';

import {
  ProviderDataError,
  ProviderHttpError,
  createCurrency,
  type FetchLike,
} from '@kurumi/finance';

import { createUsTreasuryProvider } from './provider.js';

const USD = createCurrency('USD', 2);
const EUR = createCurrency('EUR', 2);
const CSV = `Date,"1 Mo","3 Mo","1 Yr","10 Yr","30 Yr"
10/19/2026,4.11,4.27,4.50,5.15,5.48
10/16/2026,4.12,4.26,4.46,5.09,5.43
`;

function csvFetch(): FetchLike {
  return async () => new Response(CSV, { status: 200, headers: { 'content-type': 'text/csv' } });
}

describe('createUsTreasuryProvider', () => {
  it('获取最新曲线', async () => {
    const provider = createUsTreasuryProvider({
      fetchImpl: csvFetch(),
      nowMs: () => Date.UTC(2026, 9, 20),
    });
    const curve = await provider.getLatest(USD);
    expect(curve.date).toBe('2026-10-19');
    expect(curve.points).toHaveLength(5);
    expect(curve.points[0]?.tenorYears).toBeCloseTo(1 / 12, 10);
    expect(curve.points[0]?.yieldRate.toString()).toBe('0.0411');
    expect(curve.points[4]?.yieldRate.toString()).toBe('0.0548');
  });

  it('获取指定日期曲线', async () => {
    const provider = createUsTreasuryProvider({ fetchImpl: csvFetch() });
    const curve = await provider.getCurve('2026-10-16', USD);
    expect(curve.date).toBe('2026-10-16');
    expect(curve.points[3]?.yieldRate.toString()).toBe('0.0509');
  });

  it('空 CSV、列数不一致与非法 CSV 日期失败', async () => {
    const empty = createUsTreasuryProvider({
      fetchImpl: async () => new Response('Date,"1 Mo"\n', { status: 200 }),
    });
    await expect(empty.getLatest(USD)).rejects.toBeInstanceOf(ProviderDataError);

    const malformed = createUsTreasuryProvider({
      fetchImpl: async () => new Response('Date,"1 Mo"\n10/19/2026\n', { status: 200 }),
    });
    await expect(malformed.getLatest(USD)).rejects.toBeInstanceOf(ProviderDataError);

    const badDate = createUsTreasuryProvider({
      fetchImpl: async () => new Response('Date,"1 Mo"\n2026-10-19,4.11\n', { status: 200 }),
    });
    await expect(badDate.getLatest(USD)).rejects.toBeInstanceOf(ProviderDataError);

    const invalidCalendar = createUsTreasuryProvider({
      fetchImpl: async () => new Response('Date,"1 Mo"\n02/31/2026,4.11\n', { status: 200 }),
    });
    await expect(invalidCalendar.getLatest(USD)).rejects.toBeInstanceOf(ProviderDataError);

    const invalidNumber = createUsTreasuryProvider({
      fetchImpl: async () =>
        new Response('Date,"1 Mo"\n10/19/2026,not-a-number\n', { status: 200 }),
    });
    await expect(invalidNumber.getLatest(USD)).rejects.toBeInstanceOf(ProviderDataError);
  });

  it('拒绝非法选项', () => {
    expect(() => createUsTreasuryProvider({ id: ' ' })).toThrow(ProviderDataError);
    expect(() => createUsTreasuryProvider({ baseUrl: 'not a url' })).toThrow(ProviderDataError);
  });

  it('取消信号会传递到 fetch', async () => {
    const controller = new AbortController();
    controller.abort();
    let observed = false;
    const provider = createUsTreasuryProvider({
      fetchImpl: async (_input, init) => {
        observed = init?.signal?.aborted === true;
        throw new Error('aborted');
      },
    });
    await expect(provider.getLatest(USD, controller.signal)).rejects.toBeInstanceOf(
      ProviderHttpError,
    );
    expect(observed).toBe(true);
  });

  it('拒绝非 USD、缺失日期与非法日期', async () => {
    const provider = createUsTreasuryProvider({ fetchImpl: csvFetch() });
    await expect(provider.getLatest(EUR)).rejects.toBeInstanceOf(ProviderDataError);
    await expect(provider.getCurve('2026-10-18', USD)).rejects.toBeInstanceOf(ProviderDataError);
    await expect(provider.getCurve('2026/10/16', USD)).rejects.toBeInstanceOf(ProviderDataError);
  });
});
