/**
 * @file 标量 schema 单元测试。
 * @description 覆盖长度/范围边界、强转语法、字面量、包装器与定义期错误。
 * @module config/schema/scalars.test
 */

import { describe, expect, it } from 'vitest';

import { ConfigSchemaDefinitionError, ConfigValidationError } from '../errors.js';
import { ConfigIssueCode, REDACTED_VALUE, createIssue, type ConfigIssue } from '../issue.js';
import { isSchema, parseFailure, parseSuccess, type ParseOutcome } from './schema.js';
import { boolean, literal, number, oneOf, string } from './scalars.js';

function firstCode(result: ParseOutcome<unknown>): ConfigIssue['code'] {
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
    const issue = createIssue(ConfigIssueCode.Required, '$', '缺少字段');
    expect(parseFailure([issue])).toEqual({ ok: false, issues: [issue] });
    expect(() => parseFailure([])).toThrow(ConfigSchemaDefinitionError);
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
    expect(firstCode(schema.parse('a'))).toBe(ConfigIssueCode.TooSmall);
    expect(firstCode(schema.parse('abcd'))).toBe(ConfigIssueCode.TooLarge);
    expect(firstCode(schema.parse(1))).toBe(ConfigIssueCode.InvalidType);
    expect(firstCode(schema.parse(undefined))).toBe(ConfigIssueCode.Required);
  });

  it('支持 shouldTrim、isNonEmpty 与正则', () => {
    const schema = string({ shouldTrim: true, isNonEmpty: true, pattern: /^[a-z]+$/ });
    expect(schema.parseOrThrow('  abc  ')).toBe('abc');
    expect(firstCode(schema.parse('   '))).toBe(ConfigIssueCode.TooSmall);
    expect(firstCode(schema.parse('ABC'))).toBe(ConfigIssueCode.PatternMismatch);
  });

  it('isNonEmpty 在 shouldTrim=false 时也拒绝纯空白', () => {
    const schema = string({ isNonEmpty: true });
    expect(firstCode(schema.parse('   '))).toBe(ConfigIssueCode.TooSmall);
    expect(schema.parseOrThrow('  abc  ')).toBe('  abc  ');
  });

  it('移除 g/y 标志，重复解析结果稳定', () => {
    const schema = string({ pattern: /^a/g });
    expect(schema.parseOrThrow('a')).toBe('a');
    expect(schema.parseOrThrow('a')).toBe('a');
  });

  it('拒绝非法选项', () => {
    expect(() => string({ minLength: -1 })).toThrow(ConfigSchemaDefinitionError);
    expect(() => string({ minLength: 3, maxLength: 2 })).toThrow(ConfigSchemaDefinitionError);
    expect(() => string({ patternMessage: 'x' })).not.toThrow();
  });
});

describe('number', () => {
  it('接受数字与严格数字字符串，并去空白', () => {
    const schema = number({ isInteger: true, min: 1, max: 3 });
    expect(schema.parseOrThrow(2)).toBe(2);
    expect(schema.parseOrThrow(' 2 ')).toBe(2);
  });

  it('覆盖边界、整数与非有限值', () => {
    const schema = number({ isInteger: true, min: 1, max: 3 });
    expect(firstCode(schema.parse('0'))).toBe(ConfigIssueCode.TooSmall);
    expect(firstCode(schema.parse('4'))).toBe(ConfigIssueCode.TooLarge);
    expect(firstCode(schema.parse('1.5'))).toBe(ConfigIssueCode.NotInteger);
    expect(firstCode(schema.parse('1e999'))).toBe(ConfigIssueCode.NotFinite);
    expect(firstCode(schema.parse(Number.NaN))).toBe(ConfigIssueCode.NotFinite);
    expect(firstCode(schema.parse(Number.POSITIVE_INFINITY))).toBe(ConfigIssueCode.NotFinite);
  });

  it('拒绝非数字字符串与十六进制', () => {
    const schema = number();
    expect(firstCode(schema.parse('12abc'))).toBe(ConfigIssueCode.InvalidType);
    expect(firstCode(schema.parse('0x10'))).toBe(ConfigIssueCode.InvalidType);
    expect(firstCode(schema.parse(''))).toBe(ConfigIssueCode.InvalidType);
    expect(firstCode(schema.parse(true))).toBe(ConfigIssueCode.InvalidType);
  });

  it('shouldCoerceStrings=false 时只接受 number', () => {
    const schema = number({ shouldCoerceStrings: false });
    expect(firstCode(schema.parse('2'))).toBe(ConfigIssueCode.InvalidType);
    expect(schema.parseOrThrow(2)).toBe(2);
  });

  it('allowNonFinite 允许 Infinity，但仍拒绝 NaN', () => {
    const schema = number({ allowNonFinite: true });
    expect(schema.parseOrThrow(Number.POSITIVE_INFINITY)).toBe(Number.POSITIVE_INFINITY);
    expect(firstCode(schema.parse(Number.NaN))).toBe(ConfigIssueCode.NotFinite);
  });

  it('拒绝非法选项', () => {
    expect(() => number({ min: Number.NaN })).toThrow(ConfigSchemaDefinitionError);
    expect(() => number({ min: 2, max: 1 })).toThrow(ConfigSchemaDefinitionError);
  });
});

describe('boolean', () => {
  it('接受布尔值与四种字符串写法', () => {
    const schema = boolean();
    for (const input of [true, 'true', 'TRUE', '1']) {
      expect(schema.parseOrThrow(input)).toBe(true);
    }
    for (const input of [false, 'false', 'FALSE', '0']) {
      expect(schema.parseOrThrow(input)).toBe(false);
    }
  });

  it('拒绝其他字符串与关闭强转', () => {
    expect(firstCode(boolean().parse('yes'))).toBe(ConfigIssueCode.InvalidType);
    expect(firstCode(boolean({ shouldCoerceStrings: false }).parse('true'))).toBe(
      ConfigIssueCode.InvalidType,
    );
  });
});

describe('literal / oneOf', () => {
  it('字符串字面量严格匹配', () => {
    const schema = literal('production');
    expect(schema.parseOrThrow('production')).toBe('production');
    expect(firstCode(schema.parse('prod'))).toBe(ConfigIssueCode.InvalidLiteral);
  });

  it('布尔/数字字面量支持环境变量字符串强转', () => {
    expect(literal(true).parseOrThrow('true')).toBe(true);
    expect(literal(3).parseOrThrow('3')).toBe(3);
    expect(literal(0).parseOrThrow('0')).toBe(0);
  });

  it('可通过 shouldCoerceStrings=false 关闭字面量强转', () => {
    expect(firstCode(literal(3, { shouldCoerceStrings: false }).parse('3'))).toBe(
      ConfigIssueCode.InvalidLiteral,
    );
    expect(firstCode(oneOf([true], { shouldCoerceStrings: false }).parse('true'))).toBe(
      ConfigIssueCode.InvalidLiteral,
    );
  });

  it('oneOf 接受集合内字面量并拒绝集合外值', () => {
    const schema = oneOf(['debug', 'info', 'warn', 'error'] as const);
    expect(schema.parseOrThrow('warn')).toBe('warn');
    expect(firstCode(schema.parse('trace'))).toBe(ConfigIssueCode.InvalidLiteral);
  });

  it('拒绝空 oneOf 与非有限数字字面量', () => {
    expect(() => oneOf([])).toThrow(ConfigSchemaDefinitionError);
    expect(() => literal(Number.POSITIVE_INFINITY)).toThrow(ConfigSchemaDefinitionError);
  });
});

describe('包装器', () => {
  it('optional 允许 undefined，nullable 允许 null', () => {
    expect(string().optional().parseOrThrow(undefined)).toBeUndefined();
    expect(string().nullable().parseOrThrow(null)).toBeNull();
    expect(firstCode(string().nullable().parse(undefined))).toBe(ConfigIssueCode.Required);
  });

  it('default 在缺失时生效', () => {
    expect(number().optional().default(2).parseOrThrow(undefined)).toBe(2);
    expect(oneOf(['x', 'y']).default('y').parseOrThrow(undefined)).toBe('y');
    expect(string().default('safe').parseOrThrow(undefined)).toBe('safe');
  });

  it('default 立即校验默认值', () => {
    expect(() => number({ isInteger: true }).default(1.5)).toThrow(ConfigSchemaDefinitionError);
    expect(() => string().default(undefined as never)).toThrow(ConfigSchemaDefinitionError);
  });

  it('describe/secret 更新元数据且保持 kind', () => {
    const schema = string().describe('监听地址').secret();
    expect(schema.kind).toBe('string');
    expect(schema.metadata.description).toBe('监听地址');
    expect(schema.metadata.isSecret).toBe(true);
    expect(schema.redact('secret')).toBe(REDACTED_VALUE);
    expect(() => string().describe('   ')).toThrow(ConfigSchemaDefinitionError);
  });

  it('parseOrThrow 抛出聚合校验错误', () => {
    expect(() => string({ minLength: 2 }).parseOrThrow('a')).toThrow(ConfigValidationError);
  });
});
