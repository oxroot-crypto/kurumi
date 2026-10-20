/**
 * @file `@kurumi/log` 主入口。
 * @description 跨 runtime 日志基础设施；桶导出，不承载业务逻辑。
 * @module log
 */

export { LogClosedError, LogDefinitionError, LogError, LogSinkError } from './errors.js';
export { formatLogRecord, type LogFormat } from './format.js';
export {
  ALL_LOG_LEVELS,
  EMITTED_LOG_LEVELS,
  LogLevel,
  formatLogLevel,
  isEmittedLogLevel,
  isLevelEnabled,
  isLogLevel,
  parseLogLevel,
  type EmittedLogLevel,
} from './level.js';
export {
  createLogger,
  type Logger,
  type LoggerOptions,
  type LogFieldsProvider,
  type LogSinkFailure,
} from './logger.js';
export {
  EMPTY_LOG_FIELDS,
  type LogFields,
  type LogRecord,
  type LogValue,
  type ResolvedLogFields,
} from './record.js';
export { type LogSink } from './sink.js';
export {
  createConsoleSink,
  createMemorySink,
  type ConsoleLike,
  type ConsoleSinkOptions,
  type MemoryLogSink,
} from './sink/index.js';
