/**
 * @file `@kurumi/config` 主入口。
 * @description 仅做桶导出；核心不依赖任何 runtime 内置模块。
 * @module config
 */

export { MAX_JSON_DEPTH as MAX_CONFIG_DEPTH } from '@kurumi/core';

export {
  ConfigDisposedError,
  ConfigEnvMappingError,
  ConfigError,
  ConfigInterpolationError,
  ConfigSchemaDefinitionError,
  ConfigSourceError,
  ConfigSyntaxError,
  ConfigValidationError,
  type ConfigErrorOptions,
} from './errors.js';
export type { ConfigHost } from './host.js';
export {
  ConfigIssueCode,
  childPath,
  displayPath,
  formatIssues,
  REDACTED_VALUE,
  ROOT_PATH,
  type ConfigIssue,
} from './issue.js';
export { interpolateConfig, type InterpolationOptions } from './interpolate.js';
export { loadConfig, type LoadConfigOptions } from './loader.js';
export { mergeConfigLayers } from './merge.js';
export { parseJson } from './parser/json.js';
export { parseJsonc } from './parser/jsonc.js';
export {
  array,
  boolean,
  isSchema,
  literal,
  number,
  object,
  oneOf,
  record,
  Schema,
  SchemaKind,
  string,
  toJsonSchema,
  union,
  type ArraySchemaOptions,
  type BooleanSchemaOptions,
  type ConfigSchema,
  type ConfigShape,
  type Infer,
  type InferObject,
  type InferShape,
  type LiteralOptions,
  type LiteralValue,
  type NumberSchemaOptions,
  type ObjectSchemaOptions,
  type ParseFailure,
  type ParseOutcome,
  type ParseSuccess,
  type RecordSchemaOptions,
  type SchemaKind as SchemaKindType,
  type SchemaMetadata,
  type StringSchemaOptions,
  type UnknownKeysPolicy,
} from './schema/index.js';
export {
  createMemoryConfigHost,
  envSource,
  fileSource,
  normalizePosixPath,
  objectSource,
  resolvePosix,
  type EnvSourceOptions,
  type FileFormat,
  type FileParseContext,
  type FileParser,
  type FileSourceOptions,
  type MemoryConfigHostOptions,
  type ObjectSourceOptions,
} from './source/index.js';
export {
  ConfigSourceKind,
  type ConfigLayer,
  type ConfigSource,
  type ConfigSourceContext,
} from './source.js';
export {
  ConfigStoreStatus,
  createConfigStore,
  type ConfigChange,
  type ConfigChangeListener,
  type ConfigStore,
  type ConfigStoreOptions,
} from './store.js';
