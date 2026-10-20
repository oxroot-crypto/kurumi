/**
 * @file core/node/node-host 单元测试。
 * @description 覆盖真实文件读取、ENOENT 映射、路径解析与环境变量覆盖。
 * @module core/node/node-host.test
 */

import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import { FileNotFoundError, RuntimeHostError } from '../errors.js';
import { createNodeRuntimeHost } from './node-host.js';

const temporaryDirectories: string[] = [];

async function createTemporaryDirectory(): Promise<string> {
  const directory = await mkdtemp(join(tmpdir(), 'kurumi-core-'));
  temporaryDirectories.push(directory);
  return directory;
}

afterEach(async () => {
  await Promise.all(
    temporaryDirectories.splice(0).map((directory) => rm(directory, { recursive: true })),
  );
});

describe('createNodeRuntimeHost', () => {
  it('读取 UTF-8 文件并解析相对路径', async () => {
    const directory = await createTemporaryDirectory();
    await writeFile(join(directory, 'config.json'), '{"ok":true}', 'utf8');
    const host = createNodeRuntimeHost({ cwd: directory });

    expect(host.cwd).toBe(directory);
    expect(host.resolvePath(directory, 'config.json')).toBe(join(directory, 'config.json'));
    await expect(host.readTextFile(join(directory, 'config.json'))).resolves.toBe('{"ok":true}');
  });

  it('ENOENT 映射为 FileNotFoundError', async () => {
    const directory = await createTemporaryDirectory();
    const host = createNodeRuntimeHost({ cwd: directory });
    const error = await catchAsync(() => host.readTextFile(join(directory, 'missing.json')));
    expect(error).toBeInstanceOf(FileNotFoundError);
    expect((error as FileNotFoundError).path).toBe(join(directory, 'missing.json'));
  });

  it('其他读取错误包装为 RuntimeHostError', async () => {
    const host = createNodeRuntimeHost();
    const error = await catchAsync(() => host.readTextFile('/'));
    expect(error).toBeInstanceOf(RuntimeHostError);
  });

  it('环境变量可显式覆盖，缺省读取 process.env', () => {
    const explicit = createNodeRuntimeHost({ env: { KURUMI_MODE: 'test' } });
    expect(explicit.env()['KURUMI_MODE']).toBe('test');

    const live = createNodeRuntimeHost();
    expect(live.env()).toBe(process.env);
  });

  it('显式 env 是创建时快照且不可修改', () => {
    const source: Record<string, string | undefined> = { KURUMI_MODE: 'test' };
    const host = createNodeRuntimeHost({ env: source });
    source['KURUMI_MODE'] = 'changed';
    expect(host.env()['KURUMI_MODE']).toBe('test');
    expect(Object.isFrozen(host.env())).toBe(true);
  });

  it('readTextFile 响应 AbortSignal', async () => {
    const directory = await createTemporaryDirectory();
    await writeFile(join(directory, 'config.json'), '{}', 'utf8');
    const host = createNodeRuntimeHost({ cwd: directory });
    const controller = new AbortController();
    controller.abort();
    const error = await catchAsync(() =>
      host.readTextFile(join(directory, 'config.json'), controller.signal),
    );
    expect((error as Error).name).toBe('AbortError');
  });

  it('拒绝非法 cwd/env 选项', () => {
    expect(() => createNodeRuntimeHost(JSON.parse('{"cwd":1}'))).toThrow();
    expect(() => createNodeRuntimeHost(JSON.parse('{"env":null}'))).toThrow();
  });
});

async function catchAsync(run: () => Promise<unknown>): Promise<unknown> {
  try {
    return await run();
  } catch (error) {
    return error;
  }
}
