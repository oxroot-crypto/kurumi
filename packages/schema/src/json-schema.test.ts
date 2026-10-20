/**
 * @file JSON Schema 导出测试。
 * @description 覆盖标量、对象 required/additionalProperties、数组、记录、联合、可空与默认值。
 * @module schema/json-schema.test
 */

import { describe, expect, it } from 'vitest';

import { array, object, record, union } from './composites.js';
import { toJsonSchema } from './schema.js';
import { boolean, literal, number, oneOf, string } from './scalars.js';

describe('toJsonSchema', () => {
  it('导出标量约束与描述', () => {
    expect(
      string({ minLength: 1, maxLength: 8, pattern: /^[a-z]+$/ })
        .describe('名称')
        .toJsonSchema(),
    ).toEqual({
      type: 'string',
      minLength: 1,
      maxLength: 8,
      pattern: '^[a-z]+$',
      description: '名称',
    });
    expect(number({ isInteger: true, min: 1, max: 10 }).toJsonSchema()).toEqual({
      type: 'integer',
      minimum: 1,
      maximum: 10,
    });
    expect(number().toJsonSchema()).toEqual({ type: 'number' });
    expect(boolean().toJsonSchema()).toEqual({ type: 'boolean' });
  });

  it('导出字面量与联合', () => {
    expect(literal('a').toJsonSchema()).toEqual({ const: 'a' });
    expect(oneOf(['a', 'b'] as const).toJsonSchema()).toEqual({ enum: ['a', 'b'] });
    expect(union([string(), number()]).toJsonSchema()).toEqual({
      anyOf: [{ type: 'string' }, { type: 'number' }],
    });
  });

  it('对象 required 排除 optional/default，并反映未知键策略', () => {
    const schema = object({
      name: string(),
      port: number().default(3000),
      nickname: string().optional(),
    });
    expect(schema.toJsonSchema()).toEqual({
      type: 'object',
      properties: {
        name: { type: 'string' },
        port: { type: 'number', default: 3000 },
        nickname: { type: 'string' },
      },
      required: ['name'],
      additionalProperties: false,
    });

    expect(object({ a: string() }, { unknownKeys: 'passthrough' }).toJsonSchema()).toEqual({
      type: 'object',
      properties: { a: { type: 'string' } },
      required: ['a'],
      additionalProperties: true,
    });
  });

  it('nullable 包装 optional/default 时不进入 required', () => {
    const schema = object({
      optionalValue: string().optional().nullable(),
      defaultedValue: number().default(1).nullable(),
    });
    expect(schema.parse({})).toEqual({
      ok: true,
      value: { optionalValue: undefined, defaultedValue: 1 },
    });
    expect(schema.toJsonSchema()['required']).toBeUndefined();
  });

  it('missingSection=empty 的嵌套对象按解析结果决定 required', () => {
    const optionalSection = object(
      { nested: object({ value: string().optional() }, { missingSection: 'empty' }) },
      { missingSection: 'empty' },
    );
    expect(optionalSection.toJsonSchema()).toEqual({
      type: 'object',
      properties: {
        nested: {
          type: 'object',
          properties: { value: { type: 'string' } },
          additionalProperties: false,
        },
      },
      additionalProperties: false,
    });

    const requiredSection = object(
      { nested: object({ value: string() }, { missingSection: 'empty' }) },
      { missingSection: 'empty' },
    );
    expect(requiredSection.toJsonSchema()['required']).toEqual(['nested']);
  });

  it('导出数组、记录与可空类型', () => {
    expect(array(string(), { minLength: 1, maxLength: 3 }).toJsonSchema()).toEqual({
      type: 'array',
      items: { type: 'string' },
      minItems: 1,
      maxItems: 3,
    });
    expect(
      record(number(), { keyPattern: /^[a-z]+$/, minKeys: 1, maxKeys: 2 }).toJsonSchema(),
    ).toEqual({
      type: 'object',
      additionalProperties: { type: 'number' },
      propertyNames: { pattern: '^[a-z]+$' },
      minProperties: 1,
      maxProperties: 2,
    });
    expect(string().nullable().toJsonSchema()).toEqual({
      anyOf: [{ type: 'string' }, { type: 'null' }],
    });
  });

  it('导出结果递归冻结且为 JSON 安全值', () => {
    const schema = object({ nested: array(object({ value: string() })) });
    const json = schema.toJsonSchema();
    expect(Object.isFrozen(json)).toBe(true);
    expect(Object.isFrozen(json['properties'])).toBe(true);
    expect(JSON.parse(JSON.stringify(json))).toEqual(json);
  });

  it('独立函数导出与 schema 方法一致', () => {
    const schema = object({ a: string() });
    expect(toJsonSchema(schema)).toEqual(schema.toJsonSchema());
  });
});
