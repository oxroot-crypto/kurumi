/**
 * @file Vitest 配置。
 * @description 只运行 core 包的单元测试；共享测试/覆盖率选项来自仓库根配置。
 * @module core/vitest.config
 */

import { defineConfig } from 'vitest/config';

import { createTestOptions } from '../../vitest.shared.js';

export default defineConfig({
  test: createTestOptions(),
});
