/**
 * @file 配置值克隆适配层。
 * @description 复用 core 的 JSON 克隆能力，并把底层 KurumiError 包装为 ConfigError，保持配置错误口径统一。
 * @module config/config-value
 */

import { KurumiError, cloneJsonValue, type JsonValue } from '@kurumi/core';

import { ConfigError } from './errors.js';
import { ROOT_PATH } from './issue.js';

/**
 * 深拷贝 JSON 风格配置值。
 *
 * @param value - 待克隆值
 * @param path - 当前配置路径
 * @param depth - 递归深度
 * @returns 新对象/数组
 * @throws {ConfigError} 值不可 JSON 化、含危险键或超过深度上限
 */
export function cloneConfigValue(
  value: unknown,
  path: string = ROOT_PATH,
  depth: number = 0,
): JsonValue {
  try {
    return cloneJsonValue(value, path, depth);
  } catch (cause) {
    if (cause instanceof KurumiError) {
      throw new ConfigError(cause.message, { code: cause.code, cause });
    }
    throw cause;
  }
}
