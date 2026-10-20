/**
 * @file Vitest 配置。
 * @description 只运行 log 包的单元测试；共享测试/覆盖率选项来自仓库根配置。
 * @module log/vitest.config
 */

import { fileURLToPath } from 'node:url';

import { defineConfig } from 'vitest/config';

import { createTestOptions } from '../../vitest.shared.js';

const coreEntry = fileURLToPath(new URL('../core/src/index.ts', import.meta.url));
const coreNodeEntry = fileURLToPath(new URL('../core/src/node/index.ts', import.meta.url));

export default defineConfig({
  resolve: {
    alias: [
      { find: /^@kurumi\/core\/node$/, replacement: coreNodeEntry },
      { find: /^@kurumi\/core$/, replacement: coreEntry },
    ],
  },
  test: createTestOptions(),
});
