/**
 * @file Nasdaq 共享解析与选项测试。
 * @description 覆盖日期/金额解析、请求对象校验与选项边界。
 * @module nasdaq/support.test
 */

import { describe, expect, it } from 'vitest';

import { ProviderDataError } from '@kurumi/finance';

import {
  formatDate,
  parseDecimalField,
  parseFlexibleTimestamp,
  parseLongDate,
  parseSlashDate,
  requireArray,
  requireRecord,
  resolveNasdaqOptions,
} from './support.js';

describe('Nasdaq date/number parser', () => {
  it('解析日期与金额', () => {
    expect(parseLongDate('October 20, 2026', 'test')).toBe(Date.UTC(2026, 9, 20));
    expect(parseSlashDate('10/19/2026', 'date', 'test')).toBe(Date.UTC(2026, 9, 19));
    expect(formatDate(Date.UTC(2026, 9, 19))).toBe('2026-10-19');
    expect(parseDecimalField('$1,234.50', 'v', 'test')?.toString()).toBe('1234.5');
    expect(parseDecimalField('--', 'v', 'test')).toBeUndefined();
    expect(parseDecimalField('N/A', 'v', 'test')).toBeUndefined();
    expect(parseDecimalField(12.5, 'v', 'test')?.toString()).toBe('12.5');
  });

  it('解析灵活时间与回退', () => {
    const fallback = Date.UTC(2026, 0, 1);
    expect(parseFlexibleTimestamp('Oct 19, 2026', fallback, 'test')).toBe(Date.UTC(2026, 9, 19));
    expect(parseFlexibleTimestamp('AS OF OCT 19, 2026', fallback, 'test')).toBe(
      Date.UTC(2026, 9, 19),
    );
    expect(parseFlexibleTimestamp('Oct 20, 2026 4:53 AM ET', fallback, 'test')).toBe(
      Date.UTC(2026, 9, 20),
    );
    expect(parseFlexibleTimestamp('Oct 19', fallback, 'test')).toBe(Date.UTC(2026, 9, 19));
    expect(parseFlexibleTimestamp('', fallback, 'test')).toBe(fallback);
    expect(() => parseFlexibleTimestamp('not-a-date', fallback, 'test')).toThrow(ProviderDataError);
  });

  it('拒绝非法日期/金额与对象', () => {
    expect(() => parseLongDate('not-a-date', 'test')).toThrow(ProviderDataError);
    expect(() => parseLongDate('Smarch 1, 2026', 'test')).toThrow(ProviderDataError);
    expect(() => parseSlashDate('2026-10-19', 'date', 'test')).toThrow(ProviderDataError);
    expect(() => parseSlashDate('2/31/2026', 'date', 'test')).toThrow(ProviderDataError);
    expect(() => parseSlashDate('13/01/2026', 'date', 'test')).toThrow(ProviderDataError);
    expect(() => parseLongDate('February 31, 2026', 'test')).toThrow(ProviderDataError);
    expect(parseLongDate('Sept 19, 2026', 'test')).toBe(Date.UTC(2026, 8, 19));
    expect(() => parseDecimalField('abc', 'v', 'test')).toThrow(ProviderDataError);
    expect(() => parseDecimalField({}, 'v', 'test')).toThrow(ProviderDataError);
    expect(() => requireRecord(null, '$', 'test')).toThrow(ProviderDataError);
    expect(() => requireArray({}, '$', 'test')).toThrow(ProviderDataError);
  });
});

describe('resolveNasdaqOptions', () => {
  it('接受默认与覆盖选项', () => {
    const defaults = resolveNasdaqOptions();
    expect(defaults.id).toBe('nasdaq');
    expect(defaults.assetClass).toBe('stocks');
    expect(defaults.maxRows).toBe(50_000);
    const custom = resolveNasdaqOptions({
      id: 'custom',
      baseUrl: 'https://example.com',
      assetClass: 'etf',
      maxRows: 10,
      timeoutMs: 1_000,
      headers: { 'x-test': '1' },
    });
    expect(custom.id).toBe('custom');
    expect(custom.assetClass).toBe('etf');
    expect(custom.maxRows).toBe(10);
  });

  it('拒绝非法选项', () => {
    expect(() => resolveNasdaqOptions({ id: ' ' })).toThrow(ProviderDataError);
    expect(() => resolveNasdaqOptions({ baseUrl: 'not a url' })).toThrow(ProviderDataError);
    expect(() => resolveNasdaqOptions({ maxRows: 0 })).toThrow(ProviderDataError);
    expect(() => resolveNasdaqOptions({ assetClass: 'bad' as never })).toThrow(ProviderDataError);
  });
});
