/**
 * @file 组合 schema 单元测试。
 * @description 覆盖对象策略、记录/数组边界、联合分支、敏感值遮蔽与深度防护。
 * @module schema/composites.test
 */

import { describe, expect, expectTypeOf, it } from 'vitest';

import { array, object, record, union } from './composites.js';
import { SchemaDefinitionError } from './errors.js';
import { REDACTED_VALUE, SchemaIssueCode } from './issue.js';
import { type Infer, type ParseOutcome } from './schema.js';
import { boolean, number, string } from './scalars.js';

function issuesOf(result: ParseOutcome<unknown>) {
  if (result.ok) throw new Error('预期解析失败，但解析成功');
  return result.issues;
}

function firstCode(result: ParseOutcome<unknown>): string {
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
    const shape: { a: ReturnType<typeof number> } = { a: number() };
    const schema = object(shape);
    Reflect.set(shape, 'a', string());
    expect(schema.parseOrThrow({ a: 2 })).toEqual({ a: 2 });
    expect(firstCode(schema.parse({ a: 'x' }))).toBe(SchemaIssueCode.InvalidType);
  });

  it('填充默认值、保留 optional 键并应用强转开关', () => {
    const schema = object({
      host: string().default('localhost'),
      port: number({ isInteger: true, shouldCoerceStrings: true }),
      nickname: string().optional(),
    });
    const value = schema.parseOrThrow({ port: '3000' });
    expect(value).toEqual({ host: 'localhost', port: 3000, nickname: undefined });
    expect(Object.hasOwn(value, 'nickname')).toBe(true);
  });

  it('聚合多个字段错误并给出完整路径', () => {
    const nested = object({ server: object({ host: string(), port: number({ min: 1 }) }) });
    const issues = issuesOf(nested.parse({ server: { port: 0 } }));
    expect(issues.map((issue) => issue.path)).toEqual(['$.server.host', '$.server.port']);
    expect(issues.map((issue) => issue.code)).toEqual([
      SchemaIssueCode.Required,
      SchemaIssueCode.TooSmall,
    ]);
  });

  it('缺省拒绝未知键，strip/passthrough 按策略处理', () => {
    const schema = object({ a: number() });
    expect(firstCode(schema.parse({ a: 1, b: 2 }))).toBe(SchemaIssueCode.UnknownKey);
    expect(object({ a: number() }, { unknownKeys: 'strip' }).parseOrThrow({ a: 1, b: 2 })).toEqual({
      a: 1,
    });
    expect(
      object({ a: number() }, { unknownKeys: 'passthrough' }).parseOrThrow({ a: 1, b: 2 }),
    ).toEqual({ a: 1, b: 2 });
  });

  it('拒绝原型污染键且不污染全局原型', () => {
    const malicious = JSON.parse('{"a":1,"__proto__":{"polluted":true}}') as Record<
      string,
      unknown
    >;
    const issues = issuesOf(object({ a: number() }).parse(malicious));
    expect(issues[0]?.code).toBe(SchemaIssueCode.ForbiddenKey);
    expect(({} as Record<string, unknown>)['polluted']).toBeUndefined();
  });

  it('missingSection 缺省 error；empty 时启用嵌套默认值', () => {
    expect(firstCode(object({ enabled: boolean().default(false) }).parse(undefined))).toBe(
      SchemaIssueCode.Required,
    );
    expect(
      object({ enabled: boolean().default(false) }, { missingSection: 'empty' }).parseOrThrow(
        undefined,
      ),
    ).toEqual({ enabled: false });
  });

  it('定义期拒绝非法 shape、选项与非 Schema 子项', () => {
    expect(() => object(JSON.parse('null'))).toThrow(SchemaDefinitionError);
    expect(() => object({ a: JSON.parse('1') })).toThrow(SchemaDefinitionError);
    expect(() => array(JSON.parse('1'))).toThrow(SchemaDefinitionError);
    expect(() => union(JSON.parse('[1,2]'))).toThrow(SchemaDefinitionError);
  });

  it('passthrough 克隆未知键并拒绝非 JSON 值', () => {
    const schema = object({ a: number() }, { unknownKeys: 'passthrough' });
    const nested = { value: 1 };
    const parsed = schema.parseOrThrow({ a: 1, extra: nested });
    expect(parsed['extra']).toEqual(nested);
    expect(parsed['extra']).not.toBe(nested);

    const invalid = schema.parse({ a: 1, extra: () => 1 });
    expect(invalid.ok).toBe(false);
    expect(firstCode(invalid)).toBe(SchemaIssueCode.InvalidValue);
  });

  it('定义期拒绝危险字段名', () => {
    expect(() => object({ ['__proto__']: string() })).toThrow(SchemaDefinitionError);
  });

  it('对非法输入给出类型错误', () => {
    const schema = object({ a: number() });
    expect(firstCode(schema.parse([]))).toBe(SchemaIssueCode.InvalidType);
    expect(firstCode(schema.parse(undefined))).toBe(SchemaIssueCode.Required);
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
    const schema = record(number({ shouldCoerceStrings: true, min: 1 }));
    expect(schema.parseOrThrow({ a: 1, b: '2' })).toEqual({ a: 1, b: 2 });
    const issues = issuesOf(schema.parse({ a: 0, b: 'x' }));
    expect(issues.map((issue) => issue.path)).toEqual(['$.a', '$.b']);
  });

  it('应用键正则与数量约束', () => {
    const schema = record(number(), { keyPattern: /^[a-z]+$/, minKeys: 1, maxKeys: 2 });
    expect(schema.parseOrThrow({ a: 1 })).toEqual({ a: 1 });
    expect(firstCode(schema.parse({ A: 1 }))).toBe(SchemaIssueCode.PatternMismatch);
    expect(firstCode(schema.parse({}))).toBe(SchemaIssueCode.TooSmall);
    expect(firstCode(schema.parse({ a: 1, b: 2, c: 3 }))).toBe(SchemaIssueCode.TooLarge);
  });

  it('拒绝危险键', () => {
    const malicious = JSON.parse('{"__proto__":{"polluted":true}}') as Record<string, unknown>;
    expect(firstCode(record(number()).parse(malicious))).toBe(SchemaIssueCode.ForbiddenKey);
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
    const schema = array(number({ shouldCoerceStrings: true }));
    expect(schema.parseOrThrow(['1', 2])).toEqual([1, 2]);
    const issues = issuesOf(schema.parse(['1', 'x']));
    expect(issues[0]?.path).toBe('$[1]');
    expect(issues[0]?.code).toBe(SchemaIssueCode.InvalidType);
  });

  it('应用长度边界', () => {
    const schema = array(string(), { minLength: 1, maxLength: 2 });
    expect(firstCode(schema.parse([]))).toBe(SchemaIssueCode.TooSmall);
    expect(firstCode(schema.parse(['a', 'b', 'c']))).toBe(SchemaIssueCode.TooLarge);
  });

  it('拒绝稀疏数组与非数组', () => {
    const sparse: string[] = [];
    sparse.length = 2;
    sparse[1] = 'a';
    expect(firstCode(array(string()).parse(sparse))).toBe(SchemaIssueCode.InvalidValue);
    expect(firstCode(array(string()).parse({}))).toBe(SchemaIssueCode.InvalidType);
  });
});

describe('union', () => {
  it('按声明顺序返回第一个匹配分支', () => {
    const schema = union([string(), number({ shouldCoerceStrings: true })]);
    expect(schema.parseOrThrow('1')).toBe('1');
    expect(schema.parseOrThrow('2')).toBe('2');
  });

  it('全部失败时给出汇总问题与最接近分支的问题', () => {
    const schema = union([string(), object({ port: number() })]);
    const issues = issuesOf(schema.parse({ port: 'x' }));
    expect(issues[0]?.code).toBe(SchemaIssueCode.UnionMismatch);
    expect(issues.some((issue) => issue.path === '$.port')).toBe(true);
  });

  it('要求至少两个分支', () => {
    expect(() => union([string()] as const)).toThrow(SchemaDefinitionError);
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
    let schema: ReturnType<typeof object> | ReturnType<typeof number> = number();
    let value: unknown = 1;
    for (let index = 0; index < 70; index += 1) {
      schema = object({ a: schema });
      value = { a: value };
    }
    expect(firstCode(schema.parse(value))).toBe(SchemaIssueCode.MaxDepth);
  });
});
