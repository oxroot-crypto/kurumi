/**
 * @file Vitest 配置。
 * @description 测试直接解析到 core/math/finance 源码，避免依赖预构建的 dist。
 * @module binance/vitest.config
 */

import { fileURLToPath } from 'node:url';

import { defineConfig } from 'vitest/config';

import { createTestOptions } from '../../vitest.shared.js';

const coreEntry = fileURLToPath(new URL('../../packages/core/src/index.ts', import.meta.url));
const mathEntry = fileURLToPath(new URL('../../packages/math/src/index.ts', import.meta.url));
const financeEntry = fileURLToPath(new URL('../../packages/finance/src/index.ts', import.meta.url));

export default defineConfig({
  resolve: {
    alias: [
      { find: /^@kurumi\/math$/, replacement: mathEntry },
      { find: /^@kurumi\/finance$/, replacement: financeEntry },
      { find: /^@kurumi\/core$/, replacement: coreEntry },
    ],
  },
  test: createTestOptions(),
});
