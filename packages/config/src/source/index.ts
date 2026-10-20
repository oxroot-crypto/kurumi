/**
 * @file 配置来源公共入口。
 * @description 仅做桶导出，不承载业务逻辑。
 * @module config/source/index
 */

export {
  createMemoryRuntimeHost as createMemoryConfigHost,
  normalizePosixPath,
  resolvePosix,
  type MemoryRuntimeHostOptions as MemoryConfigHostOptions,
} from '@kurumi/core';
export { envSource, type EnvSourceOptions } from './env-source.js';
export {
  fileSource,
  type FileFormat,
  type FileParseContext,
  type FileParser,
  type FileSourceOptions,
} from './file-source.js';
export { objectSource, type ObjectSourceOptions } from './object-source.js';
