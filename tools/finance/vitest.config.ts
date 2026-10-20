/**
 * @file Vitest 配置。
 * @description 测试直接解析到 core/math/finance/schema/agent 源码，避免依赖预构建的 dist。
 * @module finance-tools/vitest.config
 */

import { fileURLToPath } from 'node:url';

import { defineConfig } from 'vitest/config';

import { createTestOptions } from '../../vitest.shared.js';

const coreEntry = fileURLToPath(new URL('../../packages/core/src/index.ts', import.meta.url));
const mathEntry = fileURLToPath(new URL('../../packages/math/src/index.ts', import.meta.url));
const financeEntry = fileURLToPath(new URL('../../packages/finance/src/index.ts', import.meta.url));
const schemaEntry = fileURLToPath(new URL('../../packages/schema/src/index.ts', import.meta.url));
const agentEntry = fileURLToPath(new URL('../../packages/agent/src/index.ts', import.meta.url));

export default defineConfig({
  resolve: {
    alias: [
      { find: /^@kurumi\/math$/, replacement: mathEntry },
      { find: /^@kurumi\/finance$/, replacement: financeEntry },
      { find: /^@kurumi\/schema$/, replacement: schemaEntry },
      { find: /^@kurumi\/agent$/, replacement: agentEntry },
      { find: /^@kurumi\/core$/, replacement: coreEntry },
    ],
  },
  test: createTestOptions(),
});
