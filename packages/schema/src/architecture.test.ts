/**
 * @file 架构约束测试。
 * @description 保证 schema 包保持 runtime 无关、无日志副作用且不依赖上层业务包。
 * @module schema/architecture.test
 */

import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

import { findMatching } from '../../../test-utils/architecture.js';

const SCHEMA_SRC = fileURLToPath(new URL('.', import.meta.url));
const SCHEMA_PACKAGE = fileURLToPath(new URL('../package.json', import.meta.url));

describe('runtime 隔离', () => {
  it('schema 核心代码不依赖 node: 内置模块', async () => {
    expect(await findMatching(SCHEMA_SRC, /from\s+['"]node:/)).toEqual([]);
  });

  it('schema 不依赖上层业务包，也不直接写日志', async () => {
    expect(await findMatching(SCHEMA_SRC, /@kurumi\/(?:agent|config|log|open-ai)/)).toEqual([]);
    expect(await findMatching(SCHEMA_SRC, /\bconsole\s*\./)).toEqual([]);
    const packageJson = await readFile(SCHEMA_PACKAGE, 'utf8');
    expect(packageJson).not.toContain('@kurumi/agent');
    expect(packageJson).not.toContain('@kurumi/config');
    expect(packageJson).not.toContain('@kurumi/log');
  });
});
