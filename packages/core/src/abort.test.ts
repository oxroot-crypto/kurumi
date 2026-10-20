/**
 * @file AbortSignal/AbortError 判定测试。
 * @description 覆盖 DOMException、Node ABORT_ERR 与结构化 signal。
 * @module core/abort.test
 */

import { describe, expect, it } from 'vitest';

import { isAbortError, isAbortSignal } from './abort.js';

describe('isAbortSignal', () => {
  it('识别 AbortSignal 并拒绝普通对象/原始值', () => {
    expect(isAbortSignal(new AbortController().signal)).toBe(true);
    expect(isAbortSignal({})).toBe(false);
    expect(isAbortSignal(null)).toBe(false);
    expect(isAbortSignal('signal')).toBe(false);
  });
});

describe('isAbortError', () => {
  it('识别 DOMException 与 Node ABORT_ERR', () => {
    expect(isAbortError(new DOMException('aborted', 'AbortError'))).toBe(true);
    expect(isAbortError({ code: 'ABORT_ERR' })).toBe(true);
    expect(isAbortError(new Error('boom'))).toBe(false);
  });
});
