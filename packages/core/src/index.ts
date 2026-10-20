/**
 * @file `@kurumi/core` 主入口。
 * @description 跨 runtime 基础设施；桶导出，不承载业务逻辑。
 * @module core
 */

export { isAbortError, isAbortSignal } from './abort.js';
export {
  FileNotFoundError,
  KurumiError,
  RuntimeHostError,
  type KurumiErrorOptions,
  type RuntimeHostErrorOptions,
} from './errors.js';
export {
  createMemoryRuntimeHost,
  normalizePosixPath,
  resolvePosix,
  type MemoryRuntimeHostOptions,
} from './memory-host.js';
export type { RuntimeHost } from './runtime.js';
export {
  DEFAULT_SERIALIZE_DEPTH,
  DEFAULT_SERIALIZE_ENTRIES,
  DEFAULT_SERIALIZE_STRING_LENGTH,
  isErrorLike,
  serializeError,
  toJsonSafeValue,
  type ErrorLike,
  type SerializedError,
  type SerializeErrorOptions,
  type SerializeValueOptions,
} from './serialize.js';
export {
  MAX_JSON_DEPTH,
  cloneJsonValue,
  deepFreeze,
  isDeepEqual,
  isForbiddenKey,
  isPlainObject,
  type JsonPrimitive,
  type JsonValue,
} from './value.js';
export {
  CoreValidationError,
  assertArrayMaxLength,
  assertBoolean,
  assertFiniteNumber,
  assertNonEmptyString,
  assertOneOf,
  assertPattern,
  assertPlainObject,
  assertPositiveSafeInteger,
  assertRange,
  assertSafeInteger,
  assertString,
  assertTimestampMs,
  describeValue,
  type ValidationErrorFactory,
} from './validation.js';
