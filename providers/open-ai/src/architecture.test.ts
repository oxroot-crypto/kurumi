/**
 * @file 架构约束测试。
 * @description 保证 open-ai 适配器 runtime 无关，且不依赖 config/log 或 console。
 * @module open-ai/architecture.test
 */

import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

import { findMatching } from '../../../test-utils/architecture.js';

const SOURCE_ROOT = fileURLToPath(new URL('.', import.meta.url));
const PACKAGE_JSON = fileURLToPath(new URL('../package.json', import.meta.url));

describe('runtime 隔离', () => {
  it('不依赖 node: 内置模块', async () => {
    expect(await findMatching(SOURCE_ROOT, /from\s+['"]node:/)).toEqual([]);
  });

  it('不依赖 config 或 log 包', async () => {
    expect(await findMatching(SOURCE_ROOT, /@kurumi\/(?:config|log)/)).toEqual([]);
    const packageJson = await readFile(PACKAGE_JSON, 'utf8');
    expect(packageJson).not.toContain('@kurumi/config');
    expect(packageJson).not.toContain('@kurumi/log');
  });

  it('不直接写 console 日志', async () => {
    expect(await findMatching(SOURCE_ROOT, /\bconsole\s*\./)).toEqual([]);
  });
});
