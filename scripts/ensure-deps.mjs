/**
 * @file 按需构建工作区依赖。
 * @description 包级 prebuild 先检查依赖 dist 是否存在，避免根构建重复执行 tsc。
 */
import { execFileSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = dirname(dirname(fileURLToPath(import.meta.url)));
const PNPM = process.platform === 'win32' ? 'pnpm.cmd' : 'pnpm';

/** 包名到仓库目录的映射。 */
const PACKAGE_DIRECTORIES = {
  '@kurumi/core': 'packages/core',
  '@kurumi/schema': 'packages/schema',
  '@kurumi/config': 'packages/config',
  '@kurumi/log': 'packages/log',
  '@kurumi/agent': 'packages/agent',
  '@kurumi/open-ai': 'providers/open-ai',
  '@kurumi/anthropic': 'providers/anthropic',
};

for (const packageName of process.argv.slice(2)) {
  const directory = PACKAGE_DIRECTORIES[packageName];
  if (directory === undefined) {
    throw new Error(`未知的工作区包: ${packageName}`);
  }
  if (existsSync(join(ROOT, directory, 'dist', 'index.js'))) continue;

  execFileSync(PNPM, ['--filter', packageName, 'build'], {
    cwd: ROOT,
    stdio: 'inherit',
  });
}
