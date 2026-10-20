/**
 * @file core runtime 隔离测试。
 * @description 保证核心入口不导入 Node 内置模块，Node 能力只出现在 node/ 子目录。
 * @module core/architecture.test
 */

import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

import { findNodeImports } from '../../../test-utils/architecture.js';

const CORE_SRC = fileURLToPath(new URL('.', import.meta.url));

describe('runtime 隔离', () => {
  it('core 核心代码不直接依赖 node: 内置模块', async () => {
    expect(await findNodeImports(CORE_SRC, `${CORE_SRC}node`)).toEqual([]);
  });
});
