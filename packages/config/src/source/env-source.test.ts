/**
 * @file 环境变量来源单元测试。
 * @description 覆盖前缀映射、层级分隔、数组下标、结构化值、冲突与定义期校验。
 * @module config/source/env-source.test
 */

import { describe, expect, it } from 'vitest';

import { ConfigEnvMappingError, ConfigSchemaDefinitionError } from '../errors.js';
import type { ConfigLayer, ConfigSource } from '../source.js';
import { createMemoryConfigHost } from './index.js';
import { envSource } from './env-source.js';

async function load(
  source: ConfigSource,
  env: Readonly<Record<string, string | undefined>>,
): Promise<ConfigLayer | undefined> {
  return source.load({ host: createMemoryConfigHost({ env }), cwd: '/' });
}

describe('envSource', () => {
  it('按前缀映射并小写化路径', async () => {
    const source = envSource({ prefix: 'KURUMI_' });
    await expect(
      load(source, {
        KURUMI_SERVER__PORT: '3000',
        KURUMI_SERVER__HOST: '0.0.0.0',
        OTHER_VALUE: 'ignored',
      }),
    ).resolves.toEqual({ server: { port: '3000', host: '0.0.0.0' } });
    expect(source.name).toBe('env:KURUMI_');
    expect(source.kind).toBe('env');
  });

  it('shouldLowercase=false 保留原始大小写', async () => {
    const source = envSource({ prefix: 'KURUMI_', shouldLowercase: false });
    await expect(load(source, { KURUMI_SERVER__PORT: '1' })).resolves.toEqual({
      SERVER: { PORT: '1' },
    });
  });

  it('数字路径段映射为连续数组', async () => {
    const source = envSource({ prefix: 'KURUMI_' });
    await expect(
      load(source, {
        KURUMI_TAGS__0: 'a',
        KURUMI_TAGS__1: 'b',
        KURUMI_LIST__0__NAME: 'first',
      }),
    ).resolves.toEqual({ tags: ['a', 'b'], list: [{ name: 'first' }] });
  });

  it('支持 JSON 数组/对象值', async () => {
    const source = envSource({ prefix: 'KURUMI_' });
    await expect(
      load(source, {
        KURUMI_CORS__ORIGINS: '["https://a","https://b"]',
        KURUMI_FEATURE: '{"enabled":true}',
      }),
    ).resolves.toEqual({
      cors: { origins: ['https://a', 'https://b'] },
      feature: { enabled: true },
    });
  });

  it('shouldParseStructuredValues=false 时保留原始字符串', async () => {
    const source = envSource({ prefix: 'KURUMI_', shouldParseStructuredValues: false });
    await expect(load(source, { KURUMI_TAGS: '["a"]' })).resolves.toEqual({ tags: '["a"]' });
  });

  it('非法 JSON 结构化值按普通字符串处理', async () => {
    const source = envSource({ prefix: 'KURUMI_' });
    await expect(load(source, { KURUMI_VALUE: '[not-json' })).resolves.toEqual({
      value: '[not-json',
    });
  });

  it('路径冲突时抛出 ConfigEnvMappingError', async () => {
    const source = envSource({ prefix: 'KURUMI_' });
    const error = catchErrorSync(() =>
      source.load({
        host: createMemoryConfigHost({ env: { KURUMI_SERVER: 'x', KURUMI_SERVER__PORT: '1' } }),
        cwd: '/',
      }),
    );
    expect(error).toBeInstanceOf(ConfigEnvMappingError);
    expect((error as ConfigEnvMappingError).issues[0]?.code).toBe('env_conflict');
  });

  it('数组索引不连续时抛出 env_index_gap', async () => {
    const source = envSource({ prefix: 'KURUMI_' });
    const error = catchErrorSync(() =>
      source.load({
        host: createMemoryConfigHost({ env: { KURUMI_TAGS__1: 'b' } }),
        cwd: '/',
      }),
    );
    expect(error).toBeInstanceOf(ConfigEnvMappingError);
    expect((error as ConfigEnvMappingError).issues[0]?.code).toBe('env_index_gap');
  });

  it('空路径段、根数组、危险键与超大下标均被拒绝', () => {
    const source = envSource({ prefix: 'KURUMI_' });
    expect(
      codeOf(() =>
        source.load({
          host: createMemoryConfigHost({ env: { KURUMI_A____B: 'x' } }),
          cwd: '/',
        }),
      ),
    ).toBe('env_conflict');

    const rootIndex = envSource({ prefix: 'KURUMI' });
    expect(
      codeOf(() =>
        rootIndex.load({
          host: createMemoryConfigHost({ env: { KURUMI0: 'x' } }),
          cwd: '/',
        }),
      ),
    ).toBe('invalid_root');

    expect(
      codeOf(() =>
        source.load({
          host: createMemoryConfigHost({ env: { KURUMI_CONSTRUCTOR: 'x' } }),
          cwd: '/',
        }),
      ),
    ).toBe('forbidden_key');

    expect(
      codeOf(() =>
        source.load({
          host: createMemoryConfigHost({ env: { KURUMI_TAGS__10001: 'x' } }),
          cwd: '/',
        }),
      ),
    ).toBe('env_index_gap');
  });

  it('小写化后重复键被拒绝', () => {
    const source = envSource({ prefix: 'KURUMI_' });
    expect(
      codeOf(() =>
        source.load({
          host: createMemoryConfigHost({ env: { KURUMI_FOO: 'a', KURUMI_foo: 'b' } }),
          cwd: '/',
        }),
      ),
    ).toBe('env_conflict');
  });

  it('结构化 JSON 超过深度上限时显式失败而不是栈溢出', () => {
    let deep = '"x"';
    for (let index = 0; index < 70; index += 1) deep = `[${deep}]`;
    const source = envSource({ prefix: 'KURUMI_' });
    expect(
      codeOf(() =>
        source.load({
          host: createMemoryConfigHost({ env: { KURUMI_DEEP: deep } }),
          cwd: '/',
        }),
      ),
    ).toBe('max_depth_exceeded');
  });

  it('结构化 JSON 中的危险键被拒绝', () => {
    const source = envSource({ prefix: 'KURUMI_' });
    expect(
      codeOf(() =>
        source.load({
          host: createMemoryConfigHost({
            env: { KURUMI_FEATURE: '{"__proto__":{"polluted":true}}' },
          }),
          cwd: '/',
        }),
      ),
    ).toBe('forbidden_key');
    expect(({} as Record<string, unknown>)['polluted']).toBeUndefined();
  });

  it('忽略空后缀与值缺失的变量', async () => {
    const source = envSource({ prefix: 'KURUMI_' });
    await expect(
      load(source, { KURUMI_: 'x', KURUMI_MISSING: undefined, KURUMI_OK: '1' }),
    ).resolves.toEqual({ ok: '1' });
  });

  it('自定义分隔符生效', async () => {
    const source = envSource({ prefix: 'KURUMI_', separator: '.' });
    await expect(load(source, { 'KURUMI_SERVER.PORT': '1' })).resolves.toEqual({
      server: { port: '1' },
    });
  });

  it('拒绝非法 prefix / separator / name', () => {
    expect(() => envSource({ prefix: '1BAD' })).toThrow(ConfigSchemaDefinitionError);
    expect(() => envSource({ prefix: 'KURUMI_', separator: 'A' })).toThrow(
      ConfigSchemaDefinitionError,
    );
    expect(() => envSource({ prefix: 'KURUMI_', separator: '' })).toThrow(
      ConfigSchemaDefinitionError,
    );
    expect(() => envSource({ prefix: 'KURUMI_', name: '  ' })).toThrow(ConfigSchemaDefinitionError);
  });
});

function catchErrorSync(run: () => unknown): unknown {
  try {
    run();
  } catch (error) {
    return error;
  }
  throw new Error('预期抛出错误，但调用成功');
}

function codeOf(run: () => unknown): string | undefined {
  const error = catchErrorSync(run);
  if (!(error instanceof ConfigEnvMappingError)) throw error;
  return error.issues[0]?.code;
}
