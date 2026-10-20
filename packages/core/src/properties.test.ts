/**
 * @file core JSON 值与序列化的属性测试。
 * @description 用随机 JSON 输入验证深拷贝/深比较/安全序列化的不变量。
 * @module core/properties.test
 */

import fc from 'fast-check';
import { describe, expect, it } from 'vitest';

import { toJsonSafeValue } from './serialize.js';
import { cloneJsonValue, isDeepEqual, isForbiddenKey } from './value.js';

function sanitize(value: unknown): unknown {
  if (Array.isArray(value)) return value.map((item) => sanitize(item));
  if (typeof value === 'object' && value !== null) {
    const result: Record<string, unknown> = {};
    for (const [key, item] of Object.entries(value)) {
      if (!isForbiddenKey(key)) result[key] = sanitize(item);
    }
    return result;
  }
  return value;
}

const safeJson = fc.jsonValue({ maxDepth: 5 }).map((value) => sanitize(value));

describe('JSON 值属性', () => {
  it('cloneJsonValue 对任意 JSON 值保持结构相等', () => {
    fc.assert(
      fc.property(safeJson, (value) => {
        const cloned = cloneJsonValue(value);
        expect(isDeepEqual(cloned, value)).toBe(true);
      }),
      { numRuns: 100 },
    );
  });

  it('isDeepEqual 对任意 JSON 值自反', () => {
    fc.assert(
      fc.property(safeJson, (value) => {
        expect(isDeepEqual(value, value)).toBe(true);
      }),
      { numRuns: 100 },
    );
  });

  it('toJsonSafeValue 的输出始终可被 JSON.stringify', () => {
    fc.assert(
      fc.property(safeJson, (value) => {
        const safe = toJsonSafeValue(value, { maxDepth: 4, maxEntries: 20 });
        expect(() => JSON.stringify(safe)).not.toThrow();
      }),
      { numRuns: 100 },
    );
  });
});
