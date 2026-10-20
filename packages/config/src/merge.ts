/**
 * @file 多层配置合并。
 * @description 对象递归合并、数组/标量整体替换；拒绝危险键并限制深度，防止原型污染与栈溢出。
 * @module config/merge
 */

import { MAX_JSON_DEPTH, isForbiddenKey, isPlainObject } from '@kurumi/core';

import { cloneConfigValue } from './config-value.js';
import { ConfigError } from './errors.js';
import { childPath, ROOT_PATH } from './issue.js';
import type { ConfigLayer } from './source.js';

/**
 * 按数组顺序合并多层配置，后者覆盖前者。
 *
 * @remarks
 * - 普通对象递归合并；数组、标量、`null` 整体替换。
 * - `undefined` 值视为“本层未提供”，不会覆盖已有值。
 * - 不修改任何入参。
 *
 * @param layers - 从低优先级到高优先级的配置层
 * @returns 合并后的新配置对象
 * @throws {ConfigError} 某层不是普通对象、出现危险键或嵌套超过上限
 *
 * @example
 * ```ts
 * mergeConfigLayers([{ server: { port: 3000 } }, { server: { host: '0.0.0.0' } }]);
 * // { server: { port: 3000, host: '0.0.0.0' } }
 * ```
 */
export function mergeConfigLayers(layers: readonly ConfigLayer[]): ConfigLayer {
  let merged: Record<string, unknown> = {};
  for (let index = 0; index < layers.length; index += 1) {
    const layer = layers[index];
    if (!isPlainObject(layer)) {
      throw new ConfigError(`第 ${index + 1} 层配置不是普通对象`, { code: 'invalid_layer' });
    }
    merged = mergeObjects(merged, layer, ROOT_PATH, 0);
  }
  return merged;
}

function mergeObjects(
  base: Record<string, unknown>,
  override: Record<string, unknown>,
  path: string,
  depth: number,
): Record<string, unknown> {
  if (depth > MAX_JSON_DEPTH) {
    throw new ConfigError(`配置合并嵌套深度超过 ${MAX_JSON_DEPTH} 层（路径 ${path}）`, {
      code: 'max_depth_exceeded',
    });
  }

  const result: Record<string, unknown> = { ...base };
  for (const key of Object.keys(override)) {
    const value = override[key];
    if (isForbiddenKey(key)) {
      throw new ConfigError(`配置合并遇到危险键 "${key}"（路径 ${path}）`, {
        code: 'forbidden_key',
      });
    }
    if (value === undefined) continue;

    const childPathValue = childPath(path, key);
    const existing = base[key];
    if (isPlainObject(value) && isPlainObject(existing)) {
      result[key] = mergeObjects(existing, value, childPathValue, depth + 1);
    } else {
      result[key] = cloneConfigValue(value, childPathValue, depth + 1);
    }
  }
  return result;
}
