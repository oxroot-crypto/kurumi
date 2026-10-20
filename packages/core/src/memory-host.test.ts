/**
 * @file core/memory-host 单元测试。
 * @description 覆盖 POSIX 路径解析、文件读取与环境变量快照。
 * @module core/memory-host.test
 */

import { describe, expect, it } from 'vitest';

import { FileNotFoundError } from './errors.js';
import { createMemoryRuntimeHost, normalizePosixPath, resolvePosix } from './memory-host.js';

describe('normalizePosixPath', () => {
  it('处理重复斜杠、当前目录与上级目录', () => {
    expect(normalizePosixPath('/a//b/./c')).toBe('/a/b/c');
    expect(normalizePosixPath('/a/b/../c')).toBe('/a/c');
    expect(normalizePosixPath('relative/path')).toBe('/relative/path');
    expect(normalizePosixPath('/')).toBe('/');
  });
});

describe('resolvePosix', () => {
  it('相对路径基于 baseDir，绝对路径保持绝对', () => {
    expect(resolvePosix('/app', 'config.json')).toBe('/app/config.json');
    expect(resolvePosix('/app', '/etc/kurumi.json')).toBe('/etc/kurumi.json');
    expect(resolvePosix('/app', '../shared.json')).toBe('/shared.json');
  });
});

describe('createMemoryRuntimeHost', () => {
  it('按 cwd 解析并读取内存文件', async () => {
    const host = createMemoryRuntimeHost({
      cwd: '/app',
      files: { 'config.json': '{"ok":true}', '/etc/other.json': '{}' },
    });
    expect(host.cwd).toBe('/app');
    await expect(host.readTextFile('/app/config.json')).resolves.toBe('{"ok":true}');
    await expect(host.readTextFile('/etc/other.json')).resolves.toBe('{}');
  });

  it('文件不存在时抛出 FileNotFoundError', async () => {
    const host = createMemoryRuntimeHost({ cwd: '/app' });
    await expect(host.readTextFile('/app/missing.json')).rejects.toBeInstanceOf(FileNotFoundError);
  });

  it('返回环境变量快照且不可修改', () => {
    const host = createMemoryRuntimeHost({ env: { PORT: '3000' } });
    expect(host.env()).toEqual({ PORT: '3000' });
    expect(Object.isFrozen(host.env())).toBe(true);
  });

  it('resolvePath 使用宿主路径规则', () => {
    const host = createMemoryRuntimeHost({ cwd: '/app' });
    expect(host.resolvePath('/app', 'nested/config.jsonc')).toBe('/app/nested/config.jsonc');
  });

  it('readTextFile 响应 AbortSignal', async () => {
    const host = createMemoryRuntimeHost({ files: { '/app/config.json': '{}' } });
    const controller = new AbortController();
    controller.abort();
    await expect(host.readTextFile('/app/config.json', controller.signal)).rejects.toMatchObject({
      name: 'AbortError',
    });
  });

  it('拒绝非法选项与路径参数', () => {
    expect(() => createMemoryRuntimeHost(JSON.parse('{"cwd":1}'))).toThrow();
    expect(() => createMemoryRuntimeHost(JSON.parse('{"files":null}'))).toThrow();
    expect(() => normalizePosixPath(JSON.parse('1'))).toThrow();
    expect(() => resolvePosix(JSON.parse('1'), 'a')).toThrow();
  });
});
