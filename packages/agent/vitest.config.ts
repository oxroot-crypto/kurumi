/**
 * @file Vitest 配置。
 * @description 测试直接解析到 core/schema 源码，避免依赖预构建的 dist。
 * @module agent/vitest.config
 */

import { fileURLToPath } from 'node:url';

import { defineConfig } from 'vitest/config';

import { createTestOptions } from '../../vitest.shared.js';

const coreEntry = fileURLToPath(new URL('../core/src/index.ts', import.meta.url));
const schemaEntry = fileURLToPath(new URL('../schema/src/index.ts', import.meta.url));

export default defineConfig({
  resolve: {
    alias: [
      { find: /^@kurumi\/core$/, replacement: coreEntry },
      { find: /^@kurumi\/schema$/, replacement: schemaEntry },
    ],
  },
  test: createTestOptions(),
});
