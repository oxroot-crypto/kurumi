/**
 * @file 配置加载流水线：加载来源 → 合并 → 插值 → 校验 → 冻结。
 * @description 每层来源独立加载（并行），但错误按声明顺序返回，保证结果确定性。
 * @module config/loader
 */

import { deepFreeze, isAbortSignal, isPlainObject } from '@kurumi/core';
import type { Schema as BaseSchema } from '@kurumi/schema';

import { ConfigError, ConfigSchemaDefinitionError, ConfigSourceError } from './errors.js';
import type { ConfigHost } from './host.js';
import { interpolateConfig, type InterpolationOptions } from './interpolate.js';
import { mergeConfigLayers } from './merge.js';
import { Schema } from './schema/index.js';
import { wrapSchema } from './schema/schema.js';
import type { ConfigLayer, ConfigSource, ConfigSourceContext } from './source.js';

/** 加载配置选项。 */
export interface LoadConfigOptions<TConfig extends object> {
  /**
   * 根 schema；可以是 `@kurumi/config` 的配置 schema，也可以是 `@kurumi/schema` 的通用 schema。
   * 通用 schema 会经过适配层，校验失败统一抛 `ConfigValidationError`。
   */
  readonly schema: BaseSchema<TConfig>;
  /** 配置宿主。 */
  readonly host: ConfigHost;
  /**
   * 配置来源，从低优先级到高优先级排列；缺省空数组。
   * 名称必须唯一。
   */
  readonly sources?: readonly ConfigSource[];
  /**
   * 字符串插值配置；`false` 表示关闭。
   *
   * @default undefined（启用，缺失变量报错）
   */
  readonly interpolation?: false | InterpolationOptions;
  /** 可选取消信号；来源和文件读取应尽快响应 abort。 */
  readonly signal?: AbortSignal;
}

/**
 * 加载、合并、插值并校验配置。
 *
 * @remarks
 * 返回对象会被递归冻结，调用方不能修改；需要变更请重新加载。
 *
 * @param options - schema、宿主、来源与插值选项
 * @returns 通过 schema 校验的配置，运行时已冻结
 * @throws {ConfigSchemaDefinitionError} schema/host/sources/interpolation 非法或来源名称重复
 * @throws {ConfigSourceError} 来源加载失败或返回非对象
 * @throws {ConfigInterpolationError} 插值缺失变量
 * @throws {ConfigValidationError} 配置不满足 schema
 *
 * @example
 * ```ts
 * import { envSource, fileSource, loadConfig } from '@kurumi/config';
 * import { createNodeConfigHost } from '@kurumi/config/node';
 *
 * const config = await loadConfig({
 *   schema,
 *   host: createNodeConfigHost(),
 *   sources: [
 *     fileSource({ path: 'kurumi.jsonc', isOptional: true }),
 *     envSource({ prefix: 'KURUMI_' }),
 *   ],
 * });
 * ```
 */
export async function loadConfig<TConfig extends object>(
  options: LoadConfigOptions<TConfig>,
): Promise<TConfig> {
  if (typeof options !== 'object' || options === null) {
    throw new ConfigSchemaDefinitionError('loadConfig options 必须是对象');
  }
  if (!(options.schema instanceof Schema)) {
    throw new ConfigSchemaDefinitionError('loadConfig 需要传入 @kurumi/config 创建的 Schema 实例');
  }
  assertConfigHost(options.host);
  assertSources(options.sources);
  if (
    options.interpolation !== undefined &&
    options.interpolation !== false &&
    !isPlainObject(options.interpolation)
  ) {
    throw new ConfigSchemaDefinitionError(
      `loadConfig interpolation 必须是 false 或对象（实际值: ${String(options.interpolation)}）`,
    );
  }
  if (options.signal !== undefined && !isAbortSignal(options.signal)) {
    throw new ConfigSchemaDefinitionError('loadConfig signal 必须是 AbortSignal');
  }
  options.signal?.throwIfAborted();

  // 运行期允许调用方传入通用 schema；通过适配层统一转换为配置错误类型。
  const schema = wrapSchema(options.schema);
  const sources = options.sources === undefined ? [] : [...options.sources];
  assertUniqueSourceNames(sources);
  const context: ConfigSourceContext = {
    host: options.host,
    cwd: options.host.cwd,
    ...(options.signal === undefined ? {} : { signal: options.signal }),
  };
  const layers = await loadLayers(sources, context);
  options.signal?.throwIfAborted();
  const merged = mergeConfigLayers(layers);
  const resolved =
    options.interpolation === false
      ? merged
      : interpolateConfig(merged, options.host.env(), options.interpolation ?? {});

  return deepFreeze(schema.parseOrThrow(resolved));
}

/**
 * 校验配置宿主形状。
 *
 * @param value - 待校验值
 * @throws {ConfigSchemaDefinitionError} 缺少必需的宿主成员
 */
export function assertConfigHost(value: unknown): asserts value is ConfigHost {
  if (
    typeof value !== 'object' ||
    value === null ||
    typeof Reflect.get(value, 'cwd') !== 'string' ||
    typeof Reflect.get(value, 'env') !== 'function' ||
    typeof Reflect.get(value, 'resolvePath') !== 'function' ||
    typeof Reflect.get(value, 'readTextFile') !== 'function'
  ) {
    throw new ConfigSchemaDefinitionError(
      `ConfigHost 必须提供 cwd/env/resolvePath/readTextFile（实际值: ${String(value)}）`,
    );
  }
}

/**
 * 校验来源数组与每个来源的基本形状。
 *
 * @param value - 待校验值
 * @throws {ConfigSchemaDefinitionError} 来源不是数组或成员缺少 name/load
 */
export function assertSources(
  value: unknown,
): asserts value is readonly ConfigSource[] | undefined {
  if (value === undefined) return;
  if (!Array.isArray(value)) {
    throw new ConfigSchemaDefinitionError(`sources 必须是数组（实际值: ${String(value)}）`);
  }
  for (const [index, source] of value.entries()) {
    if (
      typeof source !== 'object' ||
      source === null ||
      typeof Reflect.get(source, 'name') !== 'string' ||
      typeof Reflect.get(source, 'load') !== 'function'
    ) {
      throw new ConfigSchemaDefinitionError(
        `第 ${index + 1} 个 source 必须提供字符串 name 与 load()（实际值: ${String(source)}）`,
      );
    }
  }
}

function assertUniqueSourceNames(sources: readonly ConfigSource[]): void {
  const names = new Set<string>();
  for (const source of sources) {
    const name = source.name.trim();
    if (name === '') {
      throw new ConfigSchemaDefinitionError('配置来源名称不能为空');
    }
    if (names.has(name)) {
      throw new ConfigSchemaDefinitionError(`配置来源名称重复: "${name}"`);
    }
    names.add(name);
  }
}

async function loadLayers(
  sources: readonly ConfigSource[],
  context: ConfigSourceContext,
): Promise<readonly ConfigLayer[]> {
  const results = await Promise.allSettled(sources.map(async (source) => source.load(context)));
  const layers: ConfigLayer[] = [];
  context.signal?.throwIfAborted();

  for (let index = 0; index < results.length; index += 1) {
    const result = results[index];
    const source = sources[index];
    if (result === undefined || source === undefined) continue;

    if (result.status === 'rejected') {
      const cause: unknown = result.reason;
      if (cause instanceof ConfigError) throw cause;
      throw new ConfigSourceError(source.name, '来源加载时抛出非配置错误', { cause });
    }

    const layer = result.value;
    if (layer === undefined) continue;
    if (!isPlainObject(layer)) {
      throw new ConfigSourceError(source.name, '来源必须返回普通对象或 undefined', {
        code: 'invalid_layer',
      });
    }
    layers.push(layer);
  }

  return layers;
}
