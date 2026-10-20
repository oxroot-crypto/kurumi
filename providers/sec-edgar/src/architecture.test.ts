/**
 * @file SEC EDGAR provider runtime 隔离测试。
 * @description 保证只读适配器不依赖 Node 内置模块或 console。
 * @module sec-edgar/architecture.test
 */

import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

import { findMatching, findNodeImports } from '../../../test-utils/architecture.js';

const SOURCE_ROOT = fileURLToPath(new URL('.', import.meta.url));

describe('runtime 隔离', () => {
  it('不依赖 node: 内置模块', async () => {
    expect(await findNodeImports(SOURCE_ROOT, `${SOURCE_ROOT}node`)).toEqual([]);
  });

  it('不写 console', async () => {
    expect(await findMatching(SOURCE_ROOT, /\bconsole\./)).toEqual([]);
  });
});
