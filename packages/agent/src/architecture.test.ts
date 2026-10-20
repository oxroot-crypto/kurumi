/**
 * @file 架构约束测试。
 * @description 保证 Agent 主入口 runtime 无关，且不依赖 config/log 或 console。
 * @module agent/architecture.test
 */

import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

import { findMatching, listSourceFiles } from '../../../test-utils/architecture.js';

const AGENT_SRC = fileURLToPath(new URL('.', import.meta.url));
const AGENT_PACKAGE = fileURLToPath(new URL('../package.json', import.meta.url));

describe('runtime 隔离', () => {
  it('Agent 核心代码不依赖 node: 内置模块', async () => {
    const violations = await findMatching(AGENT_SRC, /from\s+['"]node:/);
    expect(violations).toEqual([]);
  });

  it('Agent 不依赖 config 或 log 包', async () => {
    const violations = await findMatching(AGENT_SRC, /@kurumi\/(?:config|log)/);
    expect(violations).toEqual([]);
    const packageJson = await readFile(AGENT_PACKAGE, 'utf8');
    expect(packageJson).not.toContain('@kurumi/config');
    expect(packageJson).not.toContain('@kurumi/log');
  });

  it('Agent 通过 schema 包定义工具输入，而不是重复实现 schema', async () => {
    const files = await listSourceFiles(AGENT_SRC);
    const contents = await Promise.all(files.map((file) => readFile(file, 'utf8')));
    const joined = contents.join('\n');
    expect(joined).toContain("from '@kurumi/schema'");
    expect(joined).not.toContain('function matchesLiteral');
    expect(await readFile(AGENT_PACKAGE, 'utf8')).toContain('@kurumi/schema');
  });

  it('Agent 不直接写 console 日志', async () => {
    const violations = await findMatching(AGENT_SRC, /\bconsole\s*\./);
    expect(violations).toEqual([]);
  });
});
