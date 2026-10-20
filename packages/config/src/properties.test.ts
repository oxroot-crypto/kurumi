/**
 * @file config 合并与插值的属性测试。
 * @description 验证任意 JSON 层级在合并/插值时不被修改，并保持幂等/确定性。
 * @module config/properties.test
 */

import fc from 'fast-check';
import { describe, expect, it } from 'vitest';

import { interpolateConfig } from './interpolate.js';
import { mergeConfigLayers } from './merge.js';

const FORBIDDEN_KEYS = new Set(['__proto__', 'prototype', 'constructor']);
const safeKey = fc.stringMatching(/^[a-z][a-z0-9]{0,5}$/);

function sanitize(value: unknown): unknown {
  if (Array.isArray(value)) return value.map((item) => sanitize(item));
  if (typeof value === 'object' && value !== null) {
    const result: Record<string, unknown> = {};
    for (const [key, item] of Object.entries(value)) {
      if (!FORBIDDEN_KEYS.has(key)) result[key] = sanitize(item);
    }
    return result;
  }
  return value;
}

const safeJson = fc.jsonValue({ maxDepth: 4 }).map((value) => sanitize(value));
const layer = fc.dictionary(safeKey, safeJson, { maxKeys: 8 });

describe('配置合并属性', () => {
  it('不修改输入层', () => {
    fc.assert(
      fc.property(layer, layer, (left, right) => {
        const leftBefore = structuredClone(left);
        const rightBefore = structuredClone(right);
        mergeConfigLayers([left, right]);
        expect(left).toEqual(leftBefore);
        expect(right).toEqual(rightBefore);
      }),
      { numRuns: 100 },
    );
  });

  it('同一层重复合并是幂等的', () => {
    fc.assert(
      fc.property(layer, (value) => {
        expect(mergeConfigLayers([value, value])).toEqual(mergeConfigLayers([value]));
      }),
      { numRuns: 100 },
    );
  });

  it('插值不修改输入且结果确定', () => {
    fc.assert(
      fc.property(layer, fc.dictionary(safeKey, fc.string({ maxLength: 20 })), (value, env) => {
        const before = structuredClone(value);
        const first = interpolateConfig(value, env, { shouldAllowMissing: true });
        const second = interpolateConfig(value, env, { shouldAllowMissing: true });
        expect(first).toEqual(second);
        expect(value).toEqual(before);
      }),
      { numRuns: 100 },
    );
  });
});
