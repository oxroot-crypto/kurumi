/**
 * @file 工具定义、注册表与结果归一化测试。
 * @description 覆盖合法定义、边界拒绝、schema 克隆、重名与结果契约。
 * @module agent/tool.test
 */

import { SchemaValidationError, number, object, string } from '@kurumi/schema';
import { describe, expect, expectTypeOf, it } from 'vitest';

import { AgentDefinitionError, AgentToolError } from './errors.js';
import { createTool, createToolRegistry, type ToolDefinition } from './index.js';
import { normalizeToolResult } from './tool.js';

const SCHEMA = { type: 'object', properties: { query: { type: 'string' } } } as const;

function createEcho(overrides: Partial<ToolDefinition> = {}): ToolDefinition {
  return createTool({
    name: 'echo',
    description: '回显输入',
    inputSchema: SCHEMA,
    execute: (input) => JSON.stringify(input),
    ...overrides,
  });
}

describe('createTool', () => {
  it('创建并冻结工具定义', () => {
    const tool = createTool({
      name: 'echo',
      description: '回显输入',
      inputSchema: SCHEMA,
      timeoutMs: 100,
      parseInput: (input) => input,
      execute: () => 'ok',
    });

    expect(tool.name).toBe('echo');
    expect(tool.timeoutMs).toBe(100);
    expect(Object.isFrozen(tool)).toBe(true);
    expect(Object.isFrozen(tool.inputSchema)).toBe(true);
  });

  it('克隆 inputSchema，不共享调用方引用', () => {
    const schema = { type: 'object', properties: { value: { type: 'string' } } };
    const tool = createTool({
      name: 'echo',
      description: '回显',
      inputSchema: schema,
      execute: () => 'ok',
    });

    schema.properties.value.type = 'number';

    expect(tool.inputSchema).toEqual({
      type: 'object',
      properties: { value: { type: 'string' } },
    });
  });

  it.each([
    ['名称为空', { name: '' }],
    ['名称以数字开头', { name: '1tool' }],
    ['名称为下划线开头', { name: '_tool' }],
    ['描述为空', { description: ' ' }],
    ['execute 非函数', { execute: 1 }],
    ['parseInput 非函数', { parseInput: 1 }],
    ['timeoutMs 为 0', { timeoutMs: 0 }],
    ['timeoutMs 为负', { timeoutMs: -1 }],
    ['timeoutMs 为小数', { timeoutMs: 1.5 }],
    ['inputSchema 非 JSON', { inputSchema: () => 1 }],
    ['inputSchema 不是 JSON Schema 对象', { inputSchema: 'not-a-schema' }],
  ] as ReadonlyArray<readonly [string, Record<string, unknown>]>)('拒绝%s', (_name, overrides) => {
    expect(() =>
      createTool({
        name: 'echo',
        description: '回显输入',
        inputSchema: SCHEMA,
        execute: () => 'ok',
        ...overrides,
      } as ToolDefinition),
    ).toThrow(AgentDefinitionError);
  });
});

describe('schema 输入', () => {
  it('从 schema 导出 JSON Schema 并自动校验输入', () => {
    const tool = createTool({
      name: 'echo',
      description: '回显',
      inputSchema: object({
        query: string({ isNonEmpty: true }).describe('查询词'),
        limit: number({ isInteger: true, min: 1 }).default(10),
      }),
      execute: ({ query, limit }) => `${query}:${String(limit)}`,
    });

    expect(tool.inputSchema).toEqual({
      type: 'object',
      properties: {
        query: { type: 'string', minLength: 1, description: '查询词' },
        limit: { type: 'integer', minimum: 1, default: 10 },
      },
      required: ['query'],
      additionalProperties: false,
    });
    expect(tool.parseInput?.({ query: 'kurumi' })).toEqual({ query: 'kurumi', limit: 10 });
    expect(() => tool.parseInput?.({ query: '' })).toThrow(SchemaValidationError);
  });

  it('schema 自动推导 execute 输入类型', () => {
    createTool({
      name: 'typed',
      description: '类型推导',
      inputSchema: object({ query: string(), count: number() }),
      execute: (input) => {
        expectTypeOf(input).toEqualTypeOf<{
          readonly query: string;
          readonly count: number;
        }>();
        return input.query;
      },
    });
  });

  it('显式 parseInput 优先于 schema 默认解析', () => {
    const tool = createTool({
      name: 'echo',
      description: '回显',
      inputSchema: object({ value: string() }),
      parseInput: (input) => ({ value: String(input) }),
      execute: ({ value }) => value,
    });

    expect(tool.parseInput?.({ value: 1 } as never)).toEqual({ value: '[object Object]' });
  });
});

describe('createToolRegistry', () => {
  it('保留声明顺序并暴露描述', () => {
    const first = createEcho();
    const second = createTool({
      name: 'search',
      description: '搜索',
      inputSchema: SCHEMA,
      execute: () => 'found',
    });
    const registry = createToolRegistry([first, second]);

    expect(registry.size).toBe(2);
    expect(registry.descriptors.map((tool) => tool.name)).toEqual(['echo', 'search']);
    expect(registry.has('echo')).toBe(true);
    expect(registry.has('missing')).toBe(false);
    expect(registry.get('search')?.description).toBe('搜索');
    expect(registry.get('missing')).toBeUndefined();
    expect(Object.isFrozen(registry)).toBe(true);
    expect(Object.isFrozen(registry.descriptors)).toBe(true);
    expect(Object.isFrozen(registry.descriptors[0])).toBe(true);
  });

  it('空工具数组生成空注册表', () => {
    const registry = createToolRegistry([]);

    expect(registry.size).toBe(0);
    expect(registry.descriptors).toEqual([]);
  });

  it('拒绝重名工具', () => {
    expect(() => createToolRegistry([createEcho(), createEcho()])).toThrow(AgentDefinitionError);
  });

  it('拒绝非法定义', () => {
    expect(() =>
      createToolRegistry([
        { name: 'bad name', description: 'x', inputSchema: {}, execute: () => '' },
      ]),
    ).toThrow(AgentDefinitionError);
  });

  it('拒绝非数组输入', () => {
    expect(() => createToolRegistry(null as never)).toThrow(AgentDefinitionError);
  });
});

describe('normalizeToolResult', () => {
  it('字符串转换为成功结果', () => {
    expect(normalizeToolResult('ok', 'echo')).toEqual({ content: 'ok', isError: false });
  });

  it('结构化结果缺省 isError=false 并冻结', () => {
    const result = normalizeToolResult({ content: 'ok' }, 'echo');

    expect(result).toEqual({ content: 'ok', isError: false });
    expect(Object.isFrozen(result)).toBe(true);
  });

  it('保留 isError 与 metadata，并克隆元数据', () => {
    const metadata = { source: 'test' };
    const result = normalizeToolResult({ content: 'bad', isError: true, metadata }, 'echo');
    metadata.source = 'changed';

    expect(result.isError).toBe(true);
    expect(result.metadata).toEqual({ source: 'test' });
    expect(Object.isFrozen(result.metadata)).toBe(true);
  });

  it.each([
    ['null', null],
    ['数组', []],
    ['缺少 content', { isError: false }],
    ['content 非字符串', { content: 1 }],
    ['isError 非布尔', { content: 'x', isError: 'yes' }],
    ['metadata 非 JSON', { content: 'x', metadata: () => 1 }],
  ])('拒绝%s', (_name, value) => {
    expect(() => normalizeToolResult(value, 'echo')).toThrow(AgentToolError);
  });
});
