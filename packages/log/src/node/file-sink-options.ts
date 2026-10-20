/**
 * @file Node 文件 sink 选项与定义期校验。
 * @description 将参数校验从 createNodeFileSink 中拆出，保持创建函数短小。
 * @module log/node/file-sink-options
 */

import type { LogFormat } from '../format.js';
import { LogDefinitionError } from '../errors.js';

/** Node 文件 sink 选项。 */
export interface NodeFileSinkOptions {
  /** 目标文件路径；相对路径基于进程当前工作目录解析。 */
  readonly path: string;
  /** 输出格式，缺省 `json`（JSONL）。 */
  readonly format?: LogFormat;
  /** 是否追加；缺省 true，false 表示首次打开时截断。 */
  readonly shouldAppend?: boolean;
  /** 是否自动创建父目录，缺省 true。 */
  readonly shouldEnsureDirectory?: boolean;
}

/** 已解析并校验的文件 sink 选项。 */
export interface ResolvedNodeFileSinkOptions {
  readonly path: string;
  readonly format: LogFormat;
  readonly shouldAppend: boolean;
  readonly shouldEnsureDirectory: boolean;
}

/**
 * 解析并校验文件 sink 选项。
 *
 * @param options - 调用方选项
 * @returns 已解析选项
 * @throws {LogDefinitionError} 路径为空、格式非法或布尔选项非法
 */
export function resolveNodeFileSinkOptions(
  options: NodeFileSinkOptions,
): ResolvedNodeFileSinkOptions {
  if (typeof options !== 'object' || options === null || Array.isArray(options)) {
    throw new LogDefinitionError('文件 sink options 必须是普通对象');
  }
  if (typeof options.path !== 'string' || options.path.trim() === '') {
    throw new LogDefinitionError('文件 sink 的 path 不能为空');
  }
  const format = options.format ?? 'json';
  if (format !== 'pretty' && format !== 'json') {
    throw new LogDefinitionError(`未知日志格式: ${String(format)}`);
  }
  const shouldAppend = options.shouldAppend ?? true;
  const shouldEnsureDirectory = options.shouldEnsureDirectory ?? true;
  if (typeof shouldAppend !== 'boolean') {
    throw new LogDefinitionError(
      `文件 sink 的 shouldAppend 必须是布尔值（实际值: ${String(shouldAppend)}）`,
    );
  }
  if (typeof shouldEnsureDirectory !== 'boolean') {
    throw new LogDefinitionError(
      `文件 sink 的 shouldEnsureDirectory 必须是布尔值（实际值: ${String(shouldEnsureDirectory)}）`,
    );
  }
  return { path: options.path, format, shouldAppend, shouldEnsureDirectory };
}
