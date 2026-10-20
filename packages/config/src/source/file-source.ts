/**
 * @file 文件配置来源。
 * @description 通过 ConfigHost 读取文本，支持严格 JSON、JSONC 与自定义解析器。
 * @module config/source/file-source
 */

import { FileNotFoundError, isAbortError, isPlainObject } from '@kurumi/core';

import { ConfigError, ConfigSchemaDefinitionError, ConfigSourceError } from '../errors.js';
import { parseJson } from '../parser/json.js';
import { parseJsonc } from '../parser/jsonc.js';
import {
  ConfigSourceKind,
  type ConfigLayer,
  type ConfigSource,
  type ConfigSourceContext,
} from '../source.js';

/** 内置文件格式。 */
export type FileFormat = 'json' | 'jsonc';

/** 自定义解析器上下文。 */
export interface FileParseContext {
  /** 来源名称。 */
  readonly sourceName: string;
  /** 解析后的绝对路径。 */
  readonly path: string;
}

/** 自定义文件解析器：返回任意值，由文件来源负责校验根节点为对象。 */
export type FileParser = (text: string, context: FileParseContext) => unknown;

/** 文件来源选项。 */
export interface FileSourceOptions {
  /** 文件路径；相对路径基于 `baseDir` 或 `host.cwd`。 */
  readonly path: string;
  /** 来源唯一名称，缺省 `file:<path>`。 */
  readonly name?: string;
  /** 基准目录，缺省 `host.cwd`。 */
  readonly baseDir?: string;
  /** 内置格式；`auto` 根据扩展名判断，缺省 `auto`。仅在未提供 parser 时生效。 */
  readonly format?: FileFormat | 'auto';
  /** 文件不存在时是否跳过，缺省 false。 */
  readonly isOptional?: boolean;
  /** 自定义解析器，优先于内置 format。 */
  readonly parser?: FileParser;
}

/**
 * 创建文件配置来源。
 *
 * @param options - 路径、格式、可选性与自定义解析器
 * @returns 配置来源
 * @throws {ConfigSchemaDefinitionError} 路径或名称为空
 *
 * @example
 * ```ts
 * fileSource({ path: 'kurumi.jsonc', isOptional: true });
 * ```
 */
export function fileSource(options: FileSourceOptions): ConfigSource {
  if (typeof options !== 'object' || options === null || typeof options.path !== 'string') {
    throw new ConfigSchemaDefinitionError('fileSource path 必须是非空字符串');
  }
  if (options.path.trim() === '') {
    throw new ConfigSchemaDefinitionError('fileSource path 不能为空');
  }
  if (options.name !== undefined && typeof options.name !== 'string') {
    throw new ConfigSchemaDefinitionError(
      `fileSource name 必须是字符串（实际值: ${String(options.name)}）`,
    );
  }
  if (options.baseDir !== undefined && typeof options.baseDir !== 'string') {
    throw new ConfigSchemaDefinitionError(
      `fileSource baseDir 必须是字符串（实际值: ${String(options.baseDir)}）`,
    );
  }
  if (options.isOptional !== undefined && typeof options.isOptional !== 'boolean') {
    throw new ConfigSchemaDefinitionError(
      `fileSource isOptional 必须是布尔值（实际值: ${String(options.isOptional)}）`,
    );
  }
  if (options.parser !== undefined && typeof options.parser !== 'function') {
    throw new ConfigSchemaDefinitionError(
      `fileSource parser 必须是函数（实际值: ${String(options.parser)}）`,
    );
  }
  const name = options.name ?? `file:${options.path}`;
  if (name.trim() === '') {
    throw new ConfigSchemaDefinitionError('fileSource name 不能为空');
  }
  const format = options.format ?? 'auto';
  if (format !== 'auto' && format !== 'json' && format !== 'jsonc') {
    throw new ConfigSchemaDefinitionError(`未知文件格式: ${String(format)}`);
  }
  return new FileSource({ ...options, name, format });
}

interface ResolvedFileSourceOptions extends FileSourceOptions {
  readonly name: string;
}

class FileSource implements ConfigSource {
  readonly name: string;
  readonly kind = ConfigSourceKind.File;
  readonly #options: ResolvedFileSourceOptions;

  constructor(options: ResolvedFileSourceOptions) {
    this.name = options.name;
    this.#options = options;
  }

  async load(context: ConfigSourceContext): Promise<ConfigLayer | undefined> {
    const baseDir = this.#options.baseDir ?? context.cwd;
    const absolute = context.host.resolvePath(baseDir, this.#options.path);
    const text = await this.#read(context, absolute);
    if (text === undefined) return undefined;

    const parsed = this.#parse(text, absolute);
    if (!isPlainObject(parsed)) {
      throw new ConfigSourceError(this.name, `配置根节点必须是对象: ${absolute}`, {
        code: 'invalid_root',
      });
    }
    return parsed;
  }

  async #read(context: ConfigSourceContext, absolute: string): Promise<string | undefined> {
    context.signal?.throwIfAborted();
    try {
      const text = await context.host.readTextFile(absolute, context.signal);
      context.signal?.throwIfAborted();
      return text;
    } catch (cause) {
      if (context.signal?.aborted === true) throw abortReason(context.signal);
      if (isAbortError(cause)) throw cause;
      if (this.#options.isOptional === true && cause instanceof FileNotFoundError) return undefined;
      if (cause instanceof ConfigError) throw cause;
      throw new ConfigSourceError(this.name, `读取文件失败: ${absolute}`, { cause });
    }
  }

  #parse(text: string, absolute: string): unknown {
    const parser = this.#options.parser;
    try {
      if (parser !== undefined) {
        return parser(text, { sourceName: this.name, path: absolute });
      }
      const format = this.#resolveFormat(absolute);
      return format === 'json' ? parseJson(text, this.name) : parseJsonc(text, this.name);
    } catch (cause) {
      if (cause instanceof ConfigError) throw cause;
      throw new ConfigSourceError(this.name, `解析文件失败: ${absolute}`, {
        cause,
        code: 'parse_error',
      });
    }
  }

  #resolveFormat(absolute: string): FileFormat {
    const configured = this.#options.format ?? 'auto';
    if (configured !== 'auto') return configured;
    const lower = absolute.toLowerCase();
    if (lower.endsWith('.json')) return 'json';
    if (lower.endsWith('.jsonc')) return 'jsonc';
    throw new ConfigSourceError(
      this.name,
      `无法从扩展名推断格式: ${absolute}；请显式设置 format 或提供 parser`,
      { code: 'unknown_format' },
    );
  }
}

/** 取得取消原因；没有 reason 时创建标准 AbortError。 */
function abortReason(signal: AbortSignal): unknown {
  return signal.reason ?? new DOMException('配置来源读取已取消', 'AbortError');
}
