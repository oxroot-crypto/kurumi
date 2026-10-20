/**
 * @file finance-tools 架构约束测试。
 * @description 保证工具包不绑定具体 provider，且不包含交易执行工具。
 * @module finance-tools/architecture.test
 */

import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

import { findMatching } from '../../../test-utils/architecture.js';

const SOURCE_ROOT = fileURLToPath(new URL('.', import.meta.url));

describe('金融工具边界', () => {
  it('不导入具体 provider 包', async () => {
    expect(
      await findMatching(
        SOURCE_ROOT,
        /@kurumi\/(?:nasdaq|binance|frankfurter|us-treasury|sec-edgar|open-ai|anthropic)/,
      ),
    ).toEqual([]);
  });

  it('不包含交易执行工具', async () => {
    expect(
      await findMatching(SOURCE_ROOT, /\b(submitOrder|cancelOrder|executeOrder|placeOrder)\b/),
    ).toEqual([]);
  });
});
