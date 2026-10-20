/**
 * @file finance runtime 与账务实现隔离测试。
 * @description 保证核心入口不依赖 Node/console，且浮点超越函数只出现在固定收益定价模型。
 * @module finance/architecture.test
 */

import { basename } from 'node:path';
import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

import { findMatching, findNodeImports } from '../../../test-utils/architecture.js';

const FINANCE_SRC = fileURLToPath(new URL('.', import.meta.url));
const PROVIDERS_ROOT = fileURLToPath(new URL('../../../providers/', import.meta.url));

describe('runtime 与账务隔离', () => {
  it('核心代码不直接依赖 node: 内置模块', async () => {
    expect(await findNodeImports(FINANCE_SRC, `${FINANCE_SRC}node`)).toEqual([]);
  });

  it('核心代码不写 console', async () => {
    expect(await findMatching(FINANCE_SRC, /\bconsole\./)).toEqual([]);
  });

  it('不使用 parseFloat 进行账务解析', async () => {
    expect(await findMatching(FINANCE_SRC, /\bparseFloat\(/)).toEqual([]);
  });

  it('Math 超越函数只允许出现在固定收益定价模型', async () => {
    const files = (
      await findMatching(FINANCE_SRC, /\bMath\.(?:exp|log|sqrt|pow|sin|cos|tan|erf)\b/)
    )
      .map((file) => basename(file))
      .sort();
    expect(files).toEqual(['fixed-income.ts']);
  });

  it('provider 契约保持只读，不暴露下单/撤单/执行方法', async () => {
    const providerDir = `${FINANCE_SRC}provider`;
    expect(
      await findMatching(providerDir, /\b(submitOrder|cancelOrder|executeOrder|placeOrder)\b/),
    ).toEqual([]);
  });

  it('所有 provider 适配器保持只读，不引入交易执行方法', async () => {
    expect(
      await findMatching(PROVIDERS_ROOT, /\b(submitOrder|cancelOrder|executeOrder|placeOrder)\b/),
    ).toEqual([]);
  });
});
