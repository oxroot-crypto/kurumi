/**
 * @file 内存 RuntimeHost：跨 runtime 的可测试实现。
 * @description 使用 POSIX 风格路径；文件以绝对路径为键保存在内存中。
 * @module core/memory-host
 */

import { FileNotFoundError, KurumiError } from './errors.js';
import type { RuntimeHost } from './runtime.js';

/** 内存宿主选项。 */
export interface MemoryRuntimeHostOptions {
  /** 基准目录，缺省 `/`。 */
  readonly cwd?: string;
  /** 环境变量快照，缺省空对象。 */
  readonly env?: Readonly<Record<string, string | undefined>>;
  /** 文件内容；相对路径会基于 cwd 解析。 */
  readonly files?: Readonly<Record<string, string>>;
}

/**
 * 创建内存 RuntimeHost。
 *
 * @param options - cwd、环境变量与文件内容
 * @returns 完全内存化的 RuntimeHost
 *
 * @example
 * ```ts
 * const host = createMemoryRuntimeHost({
 *   cwd: '/app',
 *   env: { PORT: '3000' },
 *   files: { 'kurumi.json': '{"server":{}}' },
 * });
 * ```
 */
export function createMemoryRuntimeHost(options: MemoryRuntimeHostOptions = {}): RuntimeHost {
  assertMemoryHostOptions(options);
  const cwd = normalizePosixPath(options.cwd ?? '/');
  const env = Object.freeze({ ...options.env });
  const files = new Map<string, string>();
  for (const [path, content] of Object.entries(options.files ?? {})) {
    files.set(resolvePosix(cwd, path), content);
  }

  return {
    cwd,
    env: () => env,
    resolvePath: (baseDir, path) => resolvePosix(baseDir, path),
    readTextFile: async (path, signal) => {
      signal?.throwIfAborted();
      const absolute = normalizePosixPath(path);
      const content = files.get(absolute);
      if (content === undefined) throw new FileNotFoundError(absolute);
      return content;
    },
  };
}

/**
 * 解析 POSIX 风格路径。
 *
 * @param baseDir - 绝对基准目录
 * @param path - 绝对或相对路径
 * @returns 规范化后的绝对路径
 * @throws {KurumiError} 参数不是字符串
 */
export function resolvePosix(baseDir: string, path: string): string {
  if (typeof baseDir !== 'string' || typeof path !== 'string') {
    throw new KurumiError(
      `路径参数必须是字符串（baseDir=${String(baseDir)}, path=${String(path)}）`,
      { code: 'invalid_options' },
    );
  }
  if (path.startsWith('/')) return normalizePosixPath(path);
  return normalizePosixPath(`${normalizePosixPath(baseDir)}/${path}`);
}

/** 校验内存宿主选项。 */
function assertMemoryHostOptions(options: MemoryRuntimeHostOptions): void {
  if (options === null || typeof options !== 'object') {
    throw new KurumiError('createMemoryRuntimeHost options 必须是对象', {
      code: 'invalid_options',
    });
  }
  if (options.cwd !== undefined && typeof options.cwd !== 'string') {
    throw new KurumiError(`cwd 必须是字符串（实际值: ${String(options.cwd)}）`, {
      code: 'invalid_options',
    });
  }
  if (options.env !== undefined && (options.env === null || typeof options.env !== 'object')) {
    throw new KurumiError('env 必须是对象', { code: 'invalid_options' });
  }
  if (
    options.files !== undefined &&
    (options.files === null || typeof options.files !== 'object')
  ) {
    throw new KurumiError('files 必须是对象', { code: 'invalid_options' });
  }
}

/**
 * 规范化 POSIX 路径：处理 `.`、`..` 与重复斜杠。
 *
 * @param path - 待规范化路径
 * @returns 以 `/` 开头的绝对路径
 * @throws {KurumiError} path 不是字符串
 */
export function normalizePosixPath(path: string): string {
  if (typeof path !== 'string') {
    throw new KurumiError(`路径必须是字符串（实际值: ${String(path)}）`, {
      code: 'invalid_options',
    });
  }
  const segments: string[] = [];
  for (const segment of path.split('/')) {
    if (segment === '' || segment === '.') continue;
    if (segment === '..') {
      segments.pop();
      continue;
    }
    segments.push(segment);
  }
  return `/${segments.join('/')}`;
}
