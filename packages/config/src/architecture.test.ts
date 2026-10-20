/**
 * @file 架构约束测试。
 * @description 保证 core/config 的 runtime 隔离：Node 内置模块只允许出现在各自 node 子目录。
 * @module config/architecture.test
 */

import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

import { findNodeImports, listSourceFiles } from '../../../test-utils/architecture.js';

const CONFIG_SRC = fileURLToPath(new URL('.', import.meta.url));
const CORE_SRC = fileURLToPath(new URL('../../core/src/', import.meta.url));
const CONFIG_PACKAGE = fileURLToPath(new URL('../package.json', import.meta.url));

describe('runtime 隔离', () => {
  it('config 核心代码不直接依赖 node: 内置模块', async () => {
    const violations = await findNodeImports(CONFIG_SRC, `${CONFIG_SRC}node`);
    expect(violations).toEqual([]);
  });

  it('core 核心代码不直接依赖 node: 内置模块', async () => {
    const violations = await findNodeImports(CORE_SRC, `${CORE_SRC}node`);
    expect(violations).toEqual([]);
  });

  it('config 依赖 core 而不是重复实现通用基础设施', async () => {
    const files = await listSourceFiles(CONFIG_SRC);
    const contents = await Promise.all(files.map((file) => readFile(file, 'utf8')));
    const joined = contents.join('\n');
    expect(joined).toContain("from '@kurumi/core'");
    expect(joined).not.toContain('function cloneJsonValue');
    expect(joined).not.toContain('function isDeepEqual');
  });

  it('config 复用 schema 包，不重复实现 schema 核心', async () => {
    const files = await listSourceFiles(CONFIG_SRC);
    const contents = await Promise.all(files.map((file) => readFile(file, 'utf8')));
    const joined = contents.join('\n');
    expect(joined).toContain("from '@kurumi/schema'");
    expect(joined).not.toContain('class StringSchema');
    expect(joined).not.toContain('function matchesLiteral');
    expect(await readFile(CONFIG_PACKAGE, 'utf8')).toContain('@kurumi/schema');
  });

  it('config 不直接输出日志，也不依赖 log 包', async () => {
    const files = await listSourceFiles(CONFIG_SRC);
    const contents = await Promise.all(files.map((file) => readFile(file, 'utf8')));
    const joined = contents.join('\n');
    expect(joined).not.toMatch(/\bconsole\s*\./);
    expect(joined).not.toContain('@kurumi/log');
    expect(await readFile(CONFIG_PACKAGE, 'utf8')).not.toContain('@kurumi/log');
  });
});
