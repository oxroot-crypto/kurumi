/**
 * @file 架构测试共享工具。
 * @description 扫描源码文件并匹配依赖/副作用模式；仅供各包架构测试复用，不参与构建产物。
 * @module test-utils/architecture
 */

import { readdir, readFile } from 'node:fs/promises';
import { join } from 'node:path';

/**
 * 列出源码文件；排除测试文件，避免测试自身的 import 造成误报。
 *
 * @param root - 扫描根目录
 * @returns 源码文件绝对路径
 */
export async function listSourceFiles(root: string): Promise<string[]> {
  const entries = await readdir(root, { recursive: true, withFileTypes: true });
  return entries
    .filter(
      (entry) => entry.isFile() && entry.name.endsWith('.ts') && !entry.name.endsWith('.test.ts'),
    )
    .map((entry) => join(entry.parentPath, entry.name));
}

/**
 * 返回内容匹配指定正则的源码文件。
 *
 * @param root - 扫描根目录
 * @param pattern - 匹配模式
 * @returns 匹配文件路径
 */
export async function findMatching(root: string, pattern: RegExp): Promise<string[]> {
  const files = await listSourceFiles(root);
  return (
    await Promise.all(
      files.map(async (file) => {
        const content = await readFile(file, 'utf8');
        return pattern.test(content) ? file : undefined;
      }),
    )
  ).filter((file): file is string => file !== undefined);
}

/**
 * 查找除允许目录外的 `node:` 依赖。
 *
 * @param root - 扫描根目录
 * @param allowedDirectory - 允许使用 Node 内置模块的子目录（无尾斜杠）
 * @returns 违规文件路径
 */
export async function findNodeImports(root: string, allowedDirectory: string): Promise<string[]> {
  const files = await listSourceFiles(root);
  return (
    await Promise.all(
      files.map(async (file) => {
        if (file.startsWith(`${allowedDirectory}/`)) return undefined;
        const content = await readFile(file, 'utf8');
        return /from\s+['"]node:/.test(content) || /require\(\s*['"]node:/.test(content)
          ? file
          : undefined;
      }),
    )
  ).filter((file): file is string => file !== undefined);
}
