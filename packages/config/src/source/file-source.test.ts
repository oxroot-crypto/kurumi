/**
 * @file 文件来源单元测试。
 * @description 覆盖格式推断、JSON/JSONC、isOptional、根节点校验、自定义解析器与错误包装。
 * @module config/source/file-source.test
 */

import { describe, expect, it } from 'vitest';

import {
  ConfigError,
  ConfigSchemaDefinitionError,
  ConfigSourceError,
  ConfigSyntaxError,
} from '../errors.js';
import type { ConfigHost } from '../host.js';
import type { ConfigSource } from '../source.js';
import { fileSource } from './file-source.js';
import { createMemoryConfigHost } from './index.js';

async function load(source: ConfigSource, host: ConfigHost): Promise<unknown> {
  return source.load({ host, cwd: host.cwd });
}

describe('fileSource', () => {
  it('按扩展名解析 JSONC，并相对于 cwd 解析路径', async () => {
    const host = createMemoryConfigHost({
      cwd: '/app',
      files: {
        'kurumi.jsonc': '{ // comment\n "server": { "port": 3000, }, }',
      },
    });
    const source = fileSource({ path: 'kurumi.jsonc' });
    expect(source.name).toBe('file:kurumi.jsonc');
    expect(source.kind).toBe('file');
    await expect(load(source, host)).resolves.toEqual({ server: { port: 3000 } });
  });

  it('.json 使用严格解析并拒绝注释', async () => {
    const host = createMemoryConfigHost({
      files: { '/app/kurumi.json': '{"a":1 // nope\n}' },
    });
    await expect(load(fileSource({ path: '/app/kurumi.json' }), host)).rejects.toBeInstanceOf(
      ConfigSyntaxError,
    );
  });

  it('支持 BOM 与显式 format', async () => {
    const host = createMemoryConfigHost({
      files: { '/app/config.txt': '\uFEFF{"a":1,}' },
    });
    const source = fileSource({ path: '/app/config.txt', format: 'jsonc' });
    await expect(load(source, host)).resolves.toEqual({ a: 1 });
  });

  it('无法推断扩展名时抛出 unknown_format', async () => {
    const host = createMemoryConfigHost({ files: { '/app/config.txt': '{}' } });
    const error = await catchError(load(fileSource({ path: '/app/config.txt' }), host));
    expect(error).toBeInstanceOf(ConfigSourceError);
    expect((error as ConfigSourceError).code).toBe('unknown_format');
  });

  it('isOptional 文件不存在时返回 undefined', async () => {
    const host = createMemoryConfigHost({ cwd: '/app' });
    await expect(
      load(fileSource({ path: 'missing.json', isOptional: true }), host),
    ).resolves.toBeUndefined();
  });

  it('非 isOptional 文件不存在时包装为 ConfigSourceError', async () => {
    const host = createMemoryConfigHost({ cwd: '/app' });
    const error = await catchError(load(fileSource({ path: 'missing.json' }), host));
    expect(error).toBeInstanceOf(ConfigSourceError);
    expect((error as ConfigSourceError).sourceName).toBe('file:missing.json');
    expect((error as ConfigSourceError).cause).toBeDefined();
  });

  it('取消信号保持 AbortError，不包装为 ConfigSourceError', async () => {
    const host = createMemoryConfigHost({ cwd: '/app', files: { 'a.json': '{}' } });
    const controller = new AbortController();
    controller.abort();
    const error = await catchError(
      Promise.resolve(
        fileSource({ path: 'a.json' }).load({
          host,
          cwd: host.cwd,
          signal: controller.signal,
        }),
      ),
    );
    expect((error as Error).name).toBe('AbortError');
  });

  it('根节点不是对象时抛出 invalid_root', async () => {
    const host = createMemoryConfigHost({ files: { '/app/list.json': '[]' } });
    const error = await catchError(load(fileSource({ path: '/app/list.json' }), host));
    expect((error as ConfigSourceError).code).toBe('invalid_root');
  });

  it('自定义 parser 优先于 format', async () => {
    const host = createMemoryConfigHost({ files: { '/app/config.any': 'a=1' } });
    const source = fileSource({
      path: '/app/config.any',
      parser: (text) => ({ raw: text }),
    });
    await expect(load(source, host)).resolves.toEqual({ raw: 'a=1' });
  });

  it('自定义 parser 的非 ConfigError 异常被包装', async () => {
    const host = createMemoryConfigHost({ files: { '/app/config.any': 'x' } });
    const source = fileSource({
      path: '/app/config.any',
      parser: () => {
        throw new Error('bad parser');
      },
    });
    const error = await catchError(load(source, host));
    expect(error).toBeInstanceOf(ConfigSourceError);
    expect((error as ConfigSourceError).code).toBe('parse_error');
  });

  it('自定义 parser 的 ConfigError 原样抛出', async () => {
    const host = createMemoryConfigHost({ files: { '/app/config.any': 'x' } });
    const expected = new ConfigError('parser domain error', { code: 'custom' });
    const source = fileSource({
      path: '/app/config.any',
      parser: () => {
        throw expected;
      },
    });
    await expect(load(source, host)).rejects.toBe(expected);
  });

  it('baseDir 覆盖 host.cwd', async () => {
    const host = createMemoryConfigHost({
      cwd: '/other',
      files: { '/config/kurumi.json': '{"a":1}' },
    });
    const source = fileSource({ path: 'kurumi.json', baseDir: '/config' });
    await expect(load(source, host)).resolves.toEqual({ a: 1 });
  });

  it('宿主读取异常被包装为 ConfigSourceError', async () => {
    const host: ConfigHost = {
      cwd: '/app',
      env: () => ({}),
      resolvePath: (_base: string, path: string) => path,
      readTextFile: async () => {
        throw new Error('disk on fire');
      },
    };
    const error = await catchError(load(fileSource({ path: '/app/config.json' }), host));
    expect(error).toBeInstanceOf(ConfigSourceError);
    expect((error as ConfigSourceError).cause).toBeInstanceOf(Error);
  });

  it('拒绝运行期非法 format', () => {
    expect(() => fileSource({ path: 'a', format: 'yaml' as never })).toThrow(
      ConfigSchemaDefinitionError,
    );
  });

  it('语法错误使用来源名称而不是绝对路径', async () => {
    const host = createMemoryConfigHost({
      cwd: '/app',
      files: { '/app/bad.json': '{' },
    });
    const error = await catchError(load(fileSource({ path: 'bad.json' }), host));
    expect(error).toBeInstanceOf(ConfigSyntaxError);
    expect((error as ConfigSyntaxError).sourceName).toBe('file:bad.json');
  });

  it('拒绝空路径与空名称', () => {
    expect(() => fileSource({ path: '  ' })).toThrow(ConfigSchemaDefinitionError);
    expect(() => fileSource({ path: 'a.json', name: ' ' })).toThrow(ConfigSchemaDefinitionError);
  });
});

async function catchError(promise: Promise<unknown>): Promise<unknown> {
  try {
    return await promise;
  } catch (error) {
    return error;
  }
}
