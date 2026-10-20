/**
 * @file Vitest 配置。
 * @description 测试直接解析到 core/schema 源码，避免依赖预构建的 dist。
 * @module config/vitest.config
 */

import { fileURLToPath } from 'node:url';

import { defineConfig } from 'vitest/config';

import { createTestOptions } from '../../vitest.shared.js';

const coreEntry = fileURLToPath(new URL('../core/src/index.ts', import.meta.url));
const schemaEntry = fileURLToPath(new URL('../schema/src/index.ts', import.meta.url));
const coreNodeEntry = fileURLToPath(new URL('../core/src/node/index.ts', import.meta.url));

export default defineConfig({
  resolve: {
    alias: [
      { find: /^@kurumi\/core\/node$/, replacement: coreNodeEntry },
      { find: /^@kurumi\/core$/, replacement: coreEntry },
      { find: /^@kurumi\/schema$/, replacement: schemaEntry },
    ],
  },
  test: createTestOptions(),
});
