/**
 * @file 内存对象配置来源。
 * @description 适用于测试、CLI 覆盖与默认值注入；值必须为普通对象。
 * @module config/source/object-source
 */

import { isPlainObject } from '@kurumi/core';

import { ConfigSchemaDefinitionError } from '../errors.js';
import { ConfigSourceKind, type ConfigLayer, type ConfigSource } from '../source.js';

/** 内存对象来源选项。 */
export interface ObjectSourceOptions {
  /** 来源唯一名称。 */
  readonly name: string;
  /** 一层配置对象；调用方在加载期间不得修改。 */
  readonly value: ConfigLayer;
}

/**
 * 创建内存对象配置来源。
 *
 * @param options - 来源名称与配置对象
 * @returns 配置来源
 * @throws {ConfigSchemaDefinitionError} 名称为空或 value 不是普通对象
 *
 * @example
 * ```ts
 * objectSource({ name: 'cli', value: { server: { port: 8080 } } });
 * ```
 */
export function objectSource(options: ObjectSourceOptions): ConfigSource {
  if (typeof options !== 'object' || options === null || typeof options.name !== 'string') {
    throw new ConfigSchemaDefinitionError('objectSource 的 name 必须是非空字符串');
  }
  if (options.name.trim() === '') {
    throw new ConfigSchemaDefinitionError('objectSource 的 name 不能为空');
  }
  if (!isPlainObject(options.value)) {
    throw new ConfigSchemaDefinitionError('objectSource 的 value 必须是普通对象');
  }
  return {
    name: options.name,
    kind: ConfigSourceKind.Object,
    load: (context) => {
      context.signal?.throwIfAborted();
      return options.value;
    },
  };
}
