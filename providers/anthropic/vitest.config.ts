/**
 * @file Vitest 配置。
 * @description 测试直接解析到 agent/core/schema 源码，避免依赖预构建的 dist。
 * @module anthropic/vitest.config
 */

import { fileURLToPath } from 'node:url';

import { defineConfig } from 'vitest/config';

import { createTestOptions } from '../../vitest.shared.js';

const agentEntry = fileURLToPath(new URL('../../packages/agent/src/index.ts', import.meta.url));
const coreEntry = fileURLToPath(new URL('../../packages/core/src/index.ts', import.meta.url));
const schemaEntry = fileURLToPath(new URL('../../packages/schema/src/index.ts', import.meta.url));

export default defineConfig({
  resolve: {
    alias: [
      { find: /^@kurumi\/agent$/, replacement: agentEntry },
      { find: /^@kurumi\/core$/, replacement: coreEntry },
      { find: /^@kurumi\/schema$/, replacement: schemaEntry },
    ],
  },
  test: createTestOptions(),
});
