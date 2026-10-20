/**
 * @file 组合 schema 单元测试。
 * @description 覆盖对象未知键策略、记录键约束、数组边界、联合分支与敏感值遮蔽。
 * @module config/schema/composites.test
 */

import { describe, expect, expectTypeOf, it } from 'vitest';

import { ConfigSchemaDefinitionError } from '../errors.js';
import { ConfigIssueCode, REDACTED_VALUE, type ConfigIssue } from '../issue.js';
import { array, object, record, union } from './composites.js';
import type { Infer, ParseOutcome, Schema } from './schema.js';
import { boolean, number, string } from './scalars.js';

function issuesOf(result: ParseOutcome<unknown>): readonly ConfigIssue[] {
  if (result.ok) throw new Error('预期解析失败，但解析成功');
  return result.issues;
}

function firstCode(result: ParseOutcome<unknown>): ConfigIssue['code'] {
  const issue = issuesOf(result)[0];
  if (issue === undefined) throw new Error('失败结果缺少 issues');
  return issue.code;
}

describe('object', () => {
  it('从 schema 推导输出类型并保留 readonly', () => {
    const typed = object({ port: number(), name: string().optional() });
    const value: Infer<typeof typed> = { port: 1, name: undefined };
    expect(value).toEqual({ port: 1, name: undefined });
    expectTypeOf(value).toEqualTypeOf<{
      readonly port: number;
      readonly name: string | undefined;
    }>();
  });

  it('构造后修改外部 shape 不会改变 schema 行为', () => {
    const shape: { a: Schema<number> } = { a: number() };
    const schema = object(shape);
    Reflect.set(shape, 'a', string());
    expect(schema.parseOrThrow({ a: 2 })).toEqual({ a: 2 });
    expect(firstCode(schema.parse({ a: 'x' }))).toBe(ConfigIssueCode.InvalidType);
  });

  const schema = object({
    host: string().default('localhost'),
    port: number({ isInteger: true }),
    nickname: string().optional(),
  });

  it('填充默认值、保留 optional 键并应用强转', () => {
    const value = schema.parseOrThrow({ port: '3000' });
    expect(value).toEqual({ host: 'localhost', port: 3000, nickname: undefined });
    expect(Object.hasOwn(value, 'nickname')).toBe(true);
  });

  it('聚合多个字段错误并给出完整路径', () => {
    const nested = object({ server: object({ host: string(), port: number({ min: 1 }) }) });
    const issues = issuesOf(nested.parse({ server: { port: 0 } }));
    expect(issues.map((issue) => issue.path)).toEqual(['$.server.host', '$.server.port']);
    expect(issues.map((issue) => issue.code)).toEqual([
      ConfigIssueCode.Required,
      ConfigIssueCode.TooSmall,
    ]);
  });

  it('缺省拒绝未知键', () => {
    const result = schema.parse({ port: 3000, typo: true });
    expect(firstCode(result)).toBe(ConfigIssueCode.UnknownKey);
  });

  it('strip 丢弃未知键，passthrough 保留未知键', () => {
    const stripped = object({ a: number() }, { unknownKeys: 'strip' });
    expect(stripped.parseOrThrow({ a: 1, b: 2 })).toEqual({ a: 1 });

    const passed = object({ a: number() }, { unknownKeys: 'passthrough' });
    expect(passed.parseOrThrow({ a: 1, b: 2 })).toEqual({ a: 1, b: 2 });
  });

  it('拒绝原型污染键且不污染全局原型', () => {
    const malicious = JSON.parse('{"a":1,"__proto__":{"polluted":true}}') as Record<
      string,
      unknown
    >;
    const issues = issuesOf(object({ a: number() }).parse(malicious));
    expect(issues[0]?.code).toBe(ConfigIssueCode.ForbiddenKey);
    expect(({} as Record<string, unknown>)['polluted']).toBeUndefined();
  });

  it('missingSection=error 时缺失 section 按必填错误处理', () => {
    const strict = object({ enabled: boolean().default(false) }, { missingSection: 'error' });
    expect(firstCode(strict.parse(undefined))).toBe(ConfigIssueCode.Required);
  });

  it('定义期拒绝危险字段名', () => {
    expect(() => object({ ['__proto__']: string() })).toThrow(ConfigSchemaDefinitionError);
  });

  it('对非法输入给出类型错误', () => {
    expect(firstCode(schema.parse([]))).toBe(ConfigIssueCode.InvalidType);
    expect(firstCode(schema.parse(undefined))).toBe(ConfigIssueCode.Required);
  });

  it('default 对象会被深拷贝，解析结果互不共享', () => {
    const defaultValue = { retry: 3 };
    const withDefault = object({ retry: number() }).default(defaultValue);
    const first = withDefault.parseOrThrow(undefined);
    (first as { retry: number }).retry = 10;
    defaultValue.retry = 20;
    expect(withDefault.parseOrThrow(undefined).retry).toBe(3);
  });
});

describe('record', () => {
  it('解析所有值并聚合错误路径', () => {
    const schema = record(number({ min: 1 }));
    expect(schema.parseOrThrow({ a: 1, b: '2' })).toEqual({ a: 1, b: 2 });
    const issues = issuesOf(schema.parse({ a: 0, b: 'x' }));
    expect(issues.map((issue) => issue.path)).toEqual(['$.a', '$.b']);
  });

  it('应用键正则与数量约束', () => {
    const schema = record(number(), { keyPattern: /^[a-z]+$/, minKeys: 1, maxKeys: 2 });
    expect(schema.parseOrThrow({ a: 1 })).toEqual({ a: 1 });
    expect(firstCode(schema.parse({ A: 1 }))).toBe(ConfigIssueCode.PatternMismatch);
    expect(firstCode(schema.parse({}))).toBe(ConfigIssueCode.TooSmall);
    expect(firstCode(schema.parse({ a: 1, b: 2, c: 3 }))).toBe(ConfigIssueCode.TooLarge);
  });

  it('拒绝危险键', () => {
    const malicious = JSON.parse('{"__proto__":{"polluted":true}}') as Record<string, unknown>;
    expect(firstCode(record(number()).parse(malicious))).toBe(ConfigIssueCode.ForbiddenKey);
  });

  it('redact 不会因危险键替换结果对象原型', () => {
    const malicious = JSON.parse('{"__proto__":{"polluted":true},"a":1}') as Readonly<
      Record<string, number>
    >;
    const redacted = record(number()).redact(malicious) as Record<string, unknown>;
    expect(redacted['a']).toBe(1);
    expect(redacted['polluted']).toBeUndefined();
    expect(Object.hasOwn(redacted, '__proto__')).toBe(false);
  });
});

describe('array', () => {
  it('解析元素并报告下标路径', () => {
    const schema = array(number());
    expect(schema.parseOrThrow(['1', 2])).toEqual([1, 2]);
    const issues = issuesOf(schema.parse(['1', 'x']));
    expect(issues[0]?.path).toBe('$[1]');
    expect(issues[0]?.code).toBe(ConfigIssueCode.InvalidType);
  });

  it('应用长度边界', () => {
    const schema = array(string(), { minLength: 1, maxLength: 2 });
    expect(firstCode(schema.parse([]))).toBe(ConfigIssueCode.TooSmall);
    expect(firstCode(schema.parse(['a', 'b', 'c']))).toBe(ConfigIssueCode.TooLarge);
  });

  it('拒绝稀疏数组与非数组', () => {
    const sparse: string[] = [];
    sparse.length = 2;
    sparse[1] = 'a';
    expect(firstCode(array(string()).parse(sparse))).toBe(ConfigIssueCode.InvalidValue);
    expect(firstCode(array(string()).parse({}))).toBe(ConfigIssueCode.InvalidType);
  });
});

describe('union', () => {
  it('按声明顺序返回第一个匹配分支', () => {
    const schema = union([string(), number()]);
    expect(schema.parseOrThrow('1')).toBe('1');
    expect(schema.parseOrThrow('2')).toBe('2');
  });

  it('全部失败时给出汇总问题与最接近分支的问题', () => {
    const schema = union([string(), object({ port: number() })]);
    const issues = issuesOf(schema.parse({ port: 'x' }));
    expect(issues[0]?.code).toBe(ConfigIssueCode.UnionMismatch);
    expect(issues.some((issue) => issue.path === '$.port')).toBe(true);
  });

  it('要求至少两个分支', () => {
    expect(() => union([string()] as const)).toThrow(ConfigSchemaDefinitionError);
  });
});

describe('redact', () => {
  it('按 schema 结构遮蔽嵌套 secret', () => {
    const schema = object({
      name: string(),
      db: object({
        user: string(),
        password: string().secret(),
      }),
    });
    expect(schema.redact({ name: 'kurumi', db: { user: 'app', password: 'p@ss' } })).toEqual({
      name: 'kurumi',
      db: { user: 'app', password: REDACTED_VALUE },
    });
  });

  it('对象整体 secret 时遮蔽整个子树', () => {
    const schema = object({ token: string(), nested: object({ key: string() }) }).secret();
    expect(schema.redact({ token: 'a', nested: { key: 'b' } })).toBe(REDACTED_VALUE);
  });

  it('联合类型选择匹配分支的遮蔽规则', () => {
    const schema = union([object({ token: string().secret() }), string()]);
    expect(schema.redact({ token: 'a' })).toEqual({ token: REDACTED_VALUE });
  });
});

describe('深度防护', () => {
  it('超过深度上限返回 max_depth_exceeded', () => {
    let schema: Schema<unknown> = number();
    let value: unknown = 1;
    for (let index = 0; index < 70; index += 1) {
      schema = object({ a: schema });
      value = { a: value };
    }
    expect(firstCode(schema.parse(value))).toBe(ConfigIssueCode.MaxDepth);
  });
});
