/**
 * @file 各包共享的 Vitest 测试选项。
 * @description 统一测试环境、覆盖率阈值与排除规则；各包只保留自己的 alias。
 * @module vitest.shared
 */

/**
 * 创建包级测试配置。
 *
 * @returns 共享的 `test` 配置片段
 */
export function createTestOptions(): {
  readonly include: readonly string[];
  readonly environment: 'node';
  readonly restoreMocks: boolean;
  readonly coverage: {
    readonly provider: 'v8';
    readonly reporter: readonly string[];
    readonly include: readonly string[];
    readonly exclude: readonly string[];
    readonly thresholds: {
      readonly statements: number;
      readonly branches: number;
      readonly functions: number;
      readonly lines: number;
    };
  };
} {
  return {
    include: ['src/**/*.test.ts'],
    environment: 'node',
    restoreMocks: true,
    coverage: {
      provider: 'v8',
      reporter: ['text', 'lcov'],
      include: ['src/**/*.ts'],
      exclude: ['src/**/*.test.ts', 'src/**/*.test-utils.ts'],
      thresholds: { statements: 85, branches: 75, functions: 85, lines: 85 },
    },
  };
}
