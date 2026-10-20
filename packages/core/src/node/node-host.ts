/**
 * @file Node.js RuntimeHost 实现。
 * @description 唯一允许直接依赖 `node:` 内置模块的位置；核心抽象保持 runtime 无关。
 * @module core/node/node-host
 */

import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';

import { isAbortError } from '../abort.js';
import { FileNotFoundError, KurumiError, RuntimeHostError } from '../errors.js';
import type { RuntimeHost } from '../runtime.js';

/** Node 宿主选项。 */
export interface NodeRuntimeHostOptions {
  /** 基准目录，缺省 `process.cwd()`。 */
  readonly cwd?: string;
  /** 环境变量快照，缺省读取实时的 `process.env`。 */
  readonly env?: Readonly<Record<string, string | undefined>>;
}

/**
 * 创建 Node.js RuntimeHost。
 *
 * @param options - cwd 与环境变量覆盖
 * @returns 基于 `node:fs/promises` 与 `node:path` 的 RuntimeHost
 *
 * @example
 * ```ts
 * const host = createNodeRuntimeHost({ cwd: '/srv/kurumi' });
 * ```
 */
export function createNodeRuntimeHost(options: NodeRuntimeHostOptions = {}): RuntimeHost {
  assertNodeHostOptions(options);
  const cwd = options.cwd ?? process.cwd();
  // 显式 env 必须做快照，避免调用方在宿主创建后修改对象导致行为漂移。
  const env = options.env === undefined ? undefined : Object.freeze({ ...options.env });

  return {
    cwd,
    env: () => env ?? process.env,
    resolvePath: (baseDir, path) => resolve(baseDir, path),
    readTextFile: async (path, signal) => {
      try {
        return await readFile(path, {
          encoding: 'utf8',
          ...(signal === undefined ? {} : { signal }),
        });
      } catch (cause) {
        // 取消不是 I/O 失败：保留 AbortError，避免上层无法区分“用户取消”和“读取失败”。
        if (signal?.aborted === true) {
          throw signal.reason ?? new DOMException('文件读取已取消', 'AbortError');
        }
        if (isAbortError(cause)) throw cause;
        if (hasErrorCode(cause, 'ENOENT')) {
          throw new FileNotFoundError(path, cause);
        }
        throw new RuntimeHostError(`读取文件失败: ${path}`, { cause });
      }
    },
  };
}

function hasErrorCode(cause: unknown, code: string): boolean {
  try {
    return typeof cause === 'object' && cause !== null && 'code' in cause && cause.code === code;
  } catch {
    return false;
  }
}

/** 校验 Node 宿主选项。 */
function assertNodeHostOptions(options: NodeRuntimeHostOptions): void {
  if (options === null || typeof options !== 'object') {
    throw new KurumiError('createNodeRuntimeHost options 必须是对象', {
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
}
