/**
 * @file math runtime 隔离测试。
 * @description 保证数学包不依赖 Node 内置模块、不写 console。
 * @module math/architecture.test
 */

import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

import { findMatching, findNodeImports } from '../../../test-utils/architecture.js';

const MATH_SRC = fileURLToPath(new URL('.', import.meta.url));
const PACKAGE_JSON = fileURLToPath(new URL('../package.json', import.meta.url));

describe('runtime 隔离', () => {
  it('不直接依赖 node: 内置模块', async () => {
    expect(await findNodeImports(MATH_SRC, `${MATH_SRC}node`)).toEqual([]);
  });

  it('不写 console', async () => {
    expect(await findMatching(MATH_SRC, /\bconsole\./)).toEqual([]);
  });

  it('不使用 parseFloat 解析账务数值', async () => {
    expect(await findMatching(MATH_SRC, /\bparseFloat\(/)).toEqual([]);
  });

  it('不依赖 finance 或任何 provider', async () => {
    expect(
      await findMatching(
        MATH_SRC,
        /@kurumi\/(?:finance|nasdaq|binance|frankfurter|us-treasury|sec-edgar)/,
      ),
    ).toEqual([]);
    const packageJson = await readFile(PACKAGE_JSON, 'utf8');
    expect(packageJson).not.toContain('@kurumi/finance');
  });
});
