/**
 * @file 标量 schema 单元测试。
 * @description 覆盖严格默认语义、强转开关、边界、字面量、包装器、错误与敏感值遮蔽。
 * @module schema/scalars.test
 */

import { describe, expect, it } from 'vitest';

import { SchemaDefinitionError, SchemaValidationError } from './errors.js';
import { REDACTED_VALUE, SchemaIssueCode, createIssue } from './issue.js';
import { SchemaKind, isSchema, parseFailure, parseSuccess, type ParseOutcome } from './schema.js';
import { boolean, literal, number, oneOf, string } from './scalars.js';

function firstCode(result: ParseOutcome<unknown>): string {
  if (result.ok) throw new Error('预期解析失败，但解析成功');
  const issue = result.issues[0];
  if (issue === undefined) throw new Error('失败结果缺少 issues');
  return issue.code;
}

describe('schema 工具', () => {
  it('isSchema 识别 Schema 实例', () => {
    expect(isSchema(string())).toBe(true);
    expect(isSchema({ parse: () => undefined })).toBe(false);
  });

  it('parseSuccess/parseFailure 构造结果并拒绝空问题列表', () => {
    expect(parseSuccess(1)).toEqual({ ok: true, value: 1 });
    const issue = createIssue(SchemaIssueCode.Required, '$', '缺少字段');
    expect(parseFailure([issue])).toEqual({ ok: false, issues: [issue] });
    expect(() => parseFailure([])).toThrow(SchemaDefinitionError);
  });

  it('parseOrThrow 抛出聚合校验错误', () => {
    expect(() => string({ minLength: 2 }).parseOrThrow('a')).toThrow(SchemaValidationError);
  });
});

describe('string', () => {
  it('接受合法字符串并按码点计算长度', () => {
    const schema = string({ minLength: 1, maxLength: 2 });
    expect(schema.parseOrThrow('a')).toBe('a');
    expect(schema.parseOrThrow('😀')).toBe('😀');
  });

  it('覆盖长度下界、上界与类型错误', () => {
    const schema = string({ minLength: 2, maxLength: 3 });
    expect(firstCode(schema.parse('a'))).toBe(SchemaIssueCode.TooSmall);
    expect(firstCode(schema.parse('abcd'))).toBe(SchemaIssueCode.TooLarge);
    expect(firstCode(schema.parse(1))).toBe(SchemaIssueCode.InvalidType);
    expect(firstCode(schema.parse(undefined))).toBe(SchemaIssueCode.Required);
  });

  it('支持 shouldTrim、isNonEmpty 与正则', () => {
    const schema = string({ shouldTrim: true, isNonEmpty: true, pattern: /^[a-z]+$/ });
    expect(schema.parseOrThrow('  abc  ')).toBe('abc');
    expect(firstCode(schema.parse('   '))).toBe(SchemaIssueCode.TooSmall);
    expect(firstCode(schema.parse('ABC'))).toBe(SchemaIssueCode.PatternMismatch);
  });

  it('isNonEmpty 在 shouldTrim=false 时也拒绝纯空白', () => {
    const schema = string({ isNonEmpty: true });
    expect(firstCode(schema.parse('   '))).toBe(SchemaIssueCode.TooSmall);
    expect(schema.parseOrThrow('  abc  ')).toBe('  abc  ');
  });

  it('移除 g/y 标志，重复解析结果稳定', () => {
    const schema = string({ pattern: /^a/g });
    expect(schema.parseOrThrow('a')).toBe('a');
    expect(schema.parseOrThrow('a')).toBe('a');
  });

  it('拒绝非法选项', () => {
    expect(() => string({ minLength: -1 })).toThrow(SchemaDefinitionError);
    expect(() => string({ minLength: 3, maxLength: 2 })).toThrow(SchemaDefinitionError);
    expect(() => string({ patternMessage: 'x' })).not.toThrow();
    expect(() => string(JSON.parse('null'))).toThrow(SchemaDefinitionError);
    expect(() => literal('x', JSON.parse('null'))).toThrow(SchemaDefinitionError);
  });
});

describe('number', () => {
  it('默认只接受数字，不隐式强转字符串', () => {
    const schema = number({ isInteger: true, min: 1, max: 3 });
    expect(schema.parseOrThrow(2)).toBe(2);
    expect(firstCode(schema.parse('2'))).toBe(SchemaIssueCode.InvalidType);
  });

  it('shouldCoerceStrings=true 时接受严格数字字符串', () => {
    const schema = number({ isInteger: true, min: 1, max: 3, shouldCoerceStrings: true });
    expect(schema.parseOrThrow(' 2 ')).toBe(2);
    expect(firstCode(schema.parse('0'))).toBe(SchemaIssueCode.TooSmall);
    expect(firstCode(schema.parse('4'))).toBe(SchemaIssueCode.TooLarge);
    expect(firstCode(schema.parse('1.5'))).toBe(SchemaIssueCode.NotInteger);
  });

  it('拒绝非数字字符串、十六进制与非有限值', () => {
    const schema = number({ shouldCoerceStrings: true });
    expect(firstCode(schema.parse('12abc'))).toBe(SchemaIssueCode.InvalidType);
    expect(firstCode(schema.parse('0x10'))).toBe(SchemaIssueCode.InvalidType);
    expect(firstCode(schema.parse(''))).toBe(SchemaIssueCode.InvalidType);
    expect(firstCode(schema.parse('1e999'))).toBe(SchemaIssueCode.NotFinite);
    expect(firstCode(schema.parse(Number.NaN))).toBe(SchemaIssueCode.NotFinite);
    expect(firstCode(schema.parse(Number.POSITIVE_INFINITY))).toBe(SchemaIssueCode.NotFinite);
  });

  it('allowNonFinite 允许 Infinity，但仍拒绝 NaN', () => {
    const schema = number({ allowNonFinite: true });
    expect(schema.parseOrThrow(Number.POSITIVE_INFINITY)).toBe(Number.POSITIVE_INFINITY);
    expect(firstCode(schema.parse(Number.NaN))).toBe(SchemaIssueCode.NotFinite);
  });

  it('拒绝非法选项', () => {
    expect(() => number({ min: Number.NaN })).toThrow(SchemaDefinitionError);
    expect(() => number({ min: 2, max: 1 })).toThrow(SchemaDefinitionError);
  });
});

describe('boolean', () => {
  it('默认只接受布尔值', () => {
    expect(boolean().parseOrThrow(true)).toBe(true);
    expect(firstCode(boolean().parse('true'))).toBe(SchemaIssueCode.InvalidType);
  });

  it('开启强转后接受四种字符串写法', () => {
    const schema = boolean({ shouldCoerceStrings: true });
    for (const input of [true, 'true', 'TRUE', '1']) {
      expect(schema.parseOrThrow(input)).toBe(true);
    }
    for (const input of [false, 'false', 'FALSE', '0']) {
      expect(schema.parseOrThrow(input)).toBe(false);
    }
    expect(firstCode(schema.parse('yes'))).toBe(SchemaIssueCode.InvalidType);
  });
});

describe('literal / oneOf', () => {
  it('字符串字面量严格匹配', () => {
    const schema = literal('production');
    expect(schema.parseOrThrow('production')).toBe('production');
    expect(firstCode(schema.parse('prod'))).toBe(SchemaIssueCode.InvalidLiteral);
  });

  it('默认不接受数字/布尔字符串强转', () => {
    expect(firstCode(literal(true).parse('true'))).toBe(SchemaIssueCode.InvalidLiteral);
    expect(firstCode(literal(3).parse('3'))).toBe(SchemaIssueCode.InvalidLiteral);
  });

  it('开启强转后接受数字/布尔字符串', () => {
    expect(literal(true, { shouldCoerceStrings: true }).parseOrThrow('true')).toBe(true);
    expect(literal(3, { shouldCoerceStrings: true }).parseOrThrow('3')).toBe(3);
  });

  it('oneOf 接受集合内字面量并拒绝集合外值', () => {
    const schema = oneOf(['debug', 'info', 'warn', 'error'] as const);
    expect(schema.parseOrThrow('warn')).toBe('warn');
    expect(firstCode(schema.parse('trace'))).toBe(SchemaIssueCode.InvalidLiteral);
  });

  it('拒绝空 oneOf 与非有限数字字面量', () => {
    expect(() => oneOf([])).toThrow(SchemaDefinitionError);
    expect(() => literal(Number.POSITIVE_INFINITY)).toThrow(SchemaDefinitionError);
  });
});

describe('包装器', () => {
  it('optional 允许 undefined，nullable 允许 null', () => {
    expect(string().optional().parseOrThrow(undefined)).toBeUndefined();
    expect(string().nullable().parseOrThrow(null)).toBeNull();
    expect(firstCode(string().nullable().parse(undefined))).toBe(SchemaIssueCode.Required);
  });

  it('default 在缺失时生效并立即校验', () => {
    expect(number().optional().default(2).parseOrThrow(undefined)).toBe(2);
    expect(oneOf(['x', 'y']).default('y').parseOrThrow(undefined)).toBe('y');
    expect(string().default('safe').parseOrThrow(undefined)).toBe('safe');
    expect(() => number({ isInteger: true }).default(1.5)).toThrow(SchemaDefinitionError);
    expect(() => string().default(undefined as never)).toThrow(SchemaDefinitionError);
  });

  it('secret 校验错误不泄漏实际值', () => {
    const stringResult = string({ pattern: /^x$/ }).secret().parse('super-secret');
    expect(stringResult.ok).toBe(false);
    if (!stringResult.ok) {
      expect(stringResult.issues[0]?.received).toBe(REDACTED_VALUE);
      expect(JSON.stringify(stringResult)).not.toContain('super-secret');
    }

    const numberResult = number({ min: 10 }).secret().parse(1);
    expect(numberResult.ok).toBe(false);
    if (!numberResult.ok) {
      expect(numberResult.issues[0]?.received).toBe(REDACTED_VALUE);
      expect(numberResult.issues[0]?.message).not.toContain('实际为');
    }
  });

  it('secret 必填缺失保留 required 语义且不泄漏值', () => {
    const result = string().secret().parse(undefined);

    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.issues[0]?.code).toBe('required');
      expect(result.issues[0]?.message).toContain('缺少必填');
      expect(result.issues[0]?.received).toBe('undefined');
    }
  });

  it('describe/secret 更新元数据且保持 kind', () => {
    const schema = string().describe('监听地址').secret();
    expect(schema.kind).toBe(SchemaKind.String);
    expect(schema.metadata.description).toBe('监听地址');
    expect(schema.metadata.isSecret).toBe(true);
    expect(schema.redact('secret')).toBe(REDACTED_VALUE);
    expect(() => string().describe('   ')).toThrow(SchemaDefinitionError);
  });
});
