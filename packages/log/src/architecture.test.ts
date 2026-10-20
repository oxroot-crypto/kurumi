/**
 * @file 架构约束测试。
 * @description 保证 log 主入口 runtime 无关，且 config 不承担日志职责。
 * @module log/architecture.test
 */

import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

import { describe, expect, it } from 'vitest';

import { findNodeImports, listSourceFiles } from '../../../test-utils/architecture.js';

const LOG_SRC = fileURLToPath(new URL('.', import.meta.url));
const CONFIG_SRC = fileURLToPath(new URL('../../config/src/', import.meta.url));
const CONFIG_PACKAGE = fileURLToPath(new URL('../../config/package.json', import.meta.url));

describe('runtime 隔离', () => {
  it('log 核心代码不直接依赖 node: 内置模块', async () => {
    const violations = await findNodeImports(LOG_SRC, `${LOG_SRC}node`);
    expect(violations).toEqual([]);
  });

  it('log 不依赖 config 包', async () => {
    const files = await listSourceFiles(LOG_SRC);
    const contents = await Promise.all(files.map((file) => readFile(file, 'utf8')));
    expect(contents.join('\n')).not.toContain('@kurumi/config');
  });
});

describe('config 与日志解耦', () => {
  it('config 源码不直接调用 console', async () => {
    const files = await listSourceFiles(CONFIG_SRC);
    const violations = (
      await Promise.all(
        files.map(async (file) => {
          const content = await readFile(file, 'utf8');
          return /\bconsole\s*\./.test(content) ? file : undefined;
        }),
      )
    ).filter((file): file is string => file !== undefined);
    expect(violations).toEqual([]);
  });

  it('config 不声明 @kurumi/log 依赖', async () => {
    const packageJson = await readFile(CONFIG_PACKAGE, 'utf8');
    expect(packageJson).not.toContain('@kurumi/log');
    const files = await listSourceFiles(CONFIG_SRC);
    const contents = await Promise.all(files.map((file) => readFile(file, 'utf8')));
    expect(contents.join('\n')).not.toContain('@kurumi/log');
  });
});
