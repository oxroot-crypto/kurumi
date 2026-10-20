/**
 * @file Node.js 追加文件 sink。
 * @description 以 JSONL（默认）或 pretty 文本串行写入；自动创建父目录并复用文件句柄。
 * @module log/node/file-sink
 */

import { mkdir, open, type FileHandle } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';

import { LogSinkError } from '../errors.js';
import { formatLogRecord, type LogFormat } from '../format.js';
import type { LogRecord } from '../record.js';
import type { LogSink } from '../sink.js';
import {
  resolveNodeFileSinkOptions,
  type NodeFileSinkOptions,
  type ResolvedNodeFileSinkOptions,
} from './file-sink-options.js';

export type { NodeFileSinkOptions } from './file-sink-options.js';

/** Node 文件 sink。 */
export interface NodeFileSink extends LogSink {
  /** 解析后的绝对路径。 */
  readonly path: string;
  /**
   * 串行追加一条记录。
   *
   * @param record - 已冻结的日志记录
   * @returns 写入完成时 resolve
   */
  write(record: LogRecord): Promise<void>;
  /** 等待当前写入队列完成。 */
  flush(): Promise<void>;
  /** 等待队列并释放文件句柄。 */
  close(): Promise<void>;
}

type FileSinkLifecycle =
  | { readonly status: 'open' }
  | { readonly status: 'closing'; readonly promise: Promise<void> }
  | { readonly status: 'closed'; readonly promise: Promise<void> };

/**
 * 创建 Node 文件 sink。
 *
 * @remarks
 * 写入串行执行，避免多行交错；`write()` 返回的 Promise 拒绝时由 Logger 统一处理。
 *
 * @param options - 路径、格式、追加与目录策略
 * @returns 带绝对路径的 sink
 * @throws {LogDefinitionError} 路径为空、格式非法或布尔选项非法
 *
 * @example
 * ```ts
 * import { createNodeFileSink } from '@kurumi/log/node';
 * const sink = createNodeFileSink({ path: 'logs/app.jsonl' });
 * ```
 */
export function createNodeFileSink(options: NodeFileSinkOptions): NodeFileSink {
  const resolved = resolveNodeFileSinkOptions(options);
  return new FileSink(resolved, resolve(resolved.path));
}

class FileSink implements NodeFileSink {
  readonly name: string;
  readonly path: string;
  readonly #format: LogFormat;
  readonly #shouldAppend: boolean;
  readonly #shouldEnsureDirectory: boolean;
  #handle: FileHandle | undefined;
  #tail: Promise<void> = Promise.resolve();
  #lastWriteError: LogSinkError | undefined;
  #lifecycle: FileSinkLifecycle = { status: 'open' };

  constructor(options: ResolvedNodeFileSinkOptions, absolutePath: string) {
    this.name = `file:${absolutePath}`;
    this.path = absolutePath;
    this.#format = options.format;
    this.#shouldAppend = options.shouldAppend;
    this.#shouldEnsureDirectory = options.shouldEnsureDirectory;
  }

  write(record: LogRecord): Promise<void> {
    if (this.#lifecycle.status !== 'open') {
      return Promise.reject(new LogSinkError(this.name, 'sink 已关闭'));
    }
    const line = `${formatLogRecord(record, this.#format)}\n`;
    const task = (async () => {
      await this.#tail;
      const file = await this.#ensureHandle();
      await file.write(line);
    })();
    const observed = (async () => {
      try {
        await task;
        // 后续写入成功说明队列已恢复；不要让旧错误在之后的 flush 中重复出现。
        this.#lastWriteError = undefined;
      } catch (cause) {
        // 直接使用 sink 的调用方可能只调用 flush/close；保留错误直到被消费。
        const error = this.#toSinkError('写入失败', cause);
        this.#lastWriteError = error;
        throw error;
      }
    })();
    // 失败由调用方（Logger 或 write 返回值）观察；tail 自身恢复，避免后续写入被前一次失败永久阻断。
    this.#tail = (async () => {
      try {
        await observed;
      } catch {
        // observed 已向调用方抛出；这里只负责让队列继续。
      }
    })();
    return observed;
  }

  async flush(): Promise<void> {
    const lifecycle = this.#lifecycle;
    if (lifecycle.status !== 'open') {
      await lifecycle.promise;
      return;
    }
    await this.#flushTail();
  }

  close(): Promise<void> {
    const lifecycle = this.#lifecycle;
    if (lifecycle.status !== 'open') return lifecycle.promise;

    const promise = this.#closeInternal();
    this.#lifecycle = { status: 'closing', promise };
    return promise;
  }

  async #closeInternal(): Promise<void> {
    let failure: LogSinkError | undefined;
    try {
      try {
        await this.#flushTail();
      } catch (cause) {
        failure = this.#toSinkError('关闭前刷新失败', cause);
      }

      if (this.#handle !== undefined) {
        const file = this.#handle;
        this.#handle = undefined;
        try {
          await file.close();
        } catch (cause) {
          failure ??= this.#toSinkError('关闭文件失败', cause);
        }
      }
      if (failure !== undefined) throw failure;
    } finally {
      const current = this.#lifecycle;
      if (current.status === 'closing') {
        this.#lifecycle = { status: 'closed', promise: current.promise };
      }
    }
  }

  async #ensureHandle(): Promise<FileHandle> {
    if (this.#handle !== undefined) return this.#handle;
    try {
      if (this.#shouldEnsureDirectory) await mkdir(dirname(this.path), { recursive: true });
      this.#handle = await open(this.path, this.#shouldAppend ? 'a' : 'w');
      return this.#handle;
    } catch (cause) {
      throw this.#toSinkError('打开文件失败', cause);
    }
  }

  async #flushTail(): Promise<void> {
    await this.#tail;
    const failure = this.#lastWriteError;
    if (failure === undefined) return;
    this.#lastWriteError = undefined;
    throw failure;
  }

  /** 将任意底层错误包装为带 sink 名称的 LogSinkError，避免直接泄漏 fs 错误类型。 */
  #toSinkError(message: string, cause: unknown): LogSinkError {
    return cause instanceof LogSinkError ? cause : new LogSinkError(this.name, message, cause);
  }
}
