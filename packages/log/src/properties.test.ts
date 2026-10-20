/**
 * @file log 字段归一化属性测试。
 * @description 验证任意 JSON 字段输入不会抛错、遵守条目上限且结果冻结。
 * @module log/properties.test
 */

import fc from 'fast-check';
import { describe, expect, it } from 'vitest';

import { normalizeLogFields, type RecordNormalizationOptions } from './record.js';

const OPTIONS: RecordNormalizationOptions = {
  maxDepth: 4,
  maxStringLength: 32,
  maxEntries: 5,
  shouldIncludeStack: false,
};

const safeKey = fc.stringMatching(/^[a-z][a-z0-9]{0,5}$/);
const fields = fc.dictionary(safeKey, fc.jsonValue({ maxDepth: 4 }), { maxKeys: 20 });

describe('日志字段属性', () => {
  it('任意 JSON 字段都不会抛错、超限并保持冻结', () => {
    fc.assert(
      fc.property(fields, (value) => {
        const normalized = normalizeLogFields(value, OPTIONS);
        expect(Object.keys(normalized).length).toBeLessThanOrEqual(OPTIONS.maxEntries + 1);
        expect(Object.isFrozen(normalized)).toBe(true);
      }),
      { numRuns: 100 },
    );
  });
});
