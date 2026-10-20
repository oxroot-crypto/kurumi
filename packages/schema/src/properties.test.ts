/**
 * @file schema 属性测试。
 * @description 验证重复解析的确定性、不修改输入以及 JSON Schema 导出稳定。
 * @module schema/properties.test
 */

import fc from 'fast-check';
import { describe, expect, it } from 'vitest';

import { object } from './composites.js';
import { boolean, number, oneOf, string } from './scalars.js';

const safeId = fc.stringMatching(/^[a-z][a-z0-9]{0,7}$/);

const schema = object({
  id: string({ pattern: /^[a-z][a-z0-9]{0,7}$/ }),
  count: number({ isInteger: true, min: 0 }),
  flag: boolean(),
  mode: oneOf(['a', 'b', 'c'] as const),
});

describe('schema 属性', () => {
  it('合法值解析幂等且不修改输入', () => {
    fc.assert(
      fc.property(
        safeId,
        fc.nat(),
        fc.boolean(),
        fc.constantFrom('a', 'b', 'c'),
        (id, count, flag, mode) => {
          const input = { id, count, flag, mode };
          const before = structuredClone(input);
          const parsed = schema.parseOrThrow(input);
          expect(parsed).toEqual(input);
          expect(schema.parseOrThrow(parsed)).toEqual(parsed);
          expect(input).toEqual(before);
        },
      ),
      { numRuns: 100 },
    );
  });

  it('JSON Schema 导出确定且可序列化', () => {
    const jsonSchema = object({
      id: string({ minLength: 1 }),
      amount: number({ min: 0 }),
      active: boolean().default(true),
      level: oneOf(['low', 'high'] as const),
    });
    fc.assert(
      fc.property(fc.constant(null), () => {
        const first = JSON.stringify(jsonSchema.toJsonSchema());
        const second = JSON.stringify(jsonSchema.toJsonSchema());
        expect(first).toBe(second);
        expect(JSON.parse(first)).toEqual(jsonSchema.toJsonSchema());
      }),
      { numRuns: 10 },
    );
  });
});
