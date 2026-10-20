/**
 * @file 日志级别定义、顺序、过滤与解析。
 * @description 级别是日志包的领域概念；配置包只提供字符串，由调用方在此解析。
 * @module log/level
 */

/** 日志级别。 */
export const LogLevel = Object.freeze({
  /** 最细粒度，通常仅用于本地排查。 */
  Trace: 'trace',
  /** 调试信息。 */
  Debug: 'debug',
  /** 常规运行信息。 */
  Info: 'info',
  /** 可恢复异常或风险。 */
  Warn: 'warn',
  /** 请求/任务失败。 */
  Error: 'error',
  /** 进程级致命错误。 */
  Fatal: 'fatal',
  /** 仅作为阈值：关闭全部输出，不能用于写入。 */
  Silent: 'silent',
} as const);

/** 日志级别联合类型。 */
export type LogLevel = (typeof LogLevel)[keyof typeof LogLevel];

/** 可写入的日志级别（排除 `silent`）。 */
export type EmittedLogLevel = Exclude<LogLevel, 'silent'>;

/** 可写入级别，从低到高排列。 */
export const EMITTED_LOG_LEVELS = Object.freeze([
  LogLevel.Trace,
  LogLevel.Debug,
  LogLevel.Info,
  LogLevel.Warn,
  LogLevel.Error,
  LogLevel.Fatal,
] as const satisfies readonly EmittedLogLevel[]);

/** 全部级别，从低到高排列；`silent` 位于末尾。 */
export const ALL_LOG_LEVELS = Object.freeze([...EMITTED_LOG_LEVELS, LogLevel.Silent] as const);

/**
 * 级别权重；数值越大越严重。
 *
 * @remarks 采用 10 的步长，便于未来在相邻级别之间插入新级别而不重排既有权重。
 */
const LEVEL_WEIGHTS: Readonly<Record<LogLevel, number>> = Object.freeze({
  [LogLevel.Trace]: 10,
  [LogLevel.Debug]: 20,
  [LogLevel.Info]: 30,
  [LogLevel.Warn]: 40,
  [LogLevel.Error]: 50,
  [LogLevel.Fatal]: 60,
  [LogLevel.Silent]: Number.POSITIVE_INFINITY,
});

/**
 * 判断值是否为合法日志级别。
 *
 * @param value - 待判断值
 * @returns 是 `trace` / `debug` / `info` / `warn` / `error` / `fatal` / `silent` 时返回 true
 */
export function isLogLevel(value: unknown): value is LogLevel {
  return (
    typeof value === 'string' &&
    Object.hasOwn(LEVEL_WEIGHTS, value) &&
    typeof Reflect.get(LEVEL_WEIGHTS, value) === 'number'
  );
}

/**
 * 判断值是否为可写入级别（排除 `silent`）。
 *
 * @param value - 待判断值
 * @returns 是 `trace` 到 `fatal` 之一时返回 true
 */
export function isEmittedLogLevel(value: unknown): value is EmittedLogLevel {
  return isLogLevel(value) && value !== LogLevel.Silent;
}

/**
 * 解析配置或环境变量中的级别字符串。
 *
 * @remarks 大小写不敏感，允许首尾空白；未知值返回 `undefined`，由调用方决定回退策略。
 *
 * @param value - 原始字符串
 * @returns 合法级别；无法识别时返回 `undefined`
 *
 * @example
 * ```ts
 * parseLogLevel('WARN'); // 'warn'
 * parseLogLevel('verbose'); // undefined
 * ```
 */
export function parseLogLevel(value: unknown): LogLevel | undefined {
  if (typeof value !== 'string') return undefined;
  const normalized = value.trim().toLowerCase();
  return isLogLevel(normalized) ? normalized : undefined;
}

/**
 * 判断写入级别是否达到阈值。
 *
 * @param threshold - logger 阈值；`silent` 表示全部关闭
 * @param level - 待写入级别
 * @returns 达到阈值时返回 true
 */
export function isLevelEnabled(threshold: LogLevel, level: EmittedLogLevel): boolean {
  if (threshold === LogLevel.Silent) return false;
  return LEVEL_WEIGHTS[level] >= LEVEL_WEIGHTS[threshold];
}

/**
 * 获取用于文本输出的级别标签。
 *
 * @param level - 日志级别
 * @returns 全大写标签，例如 `INFO`
 */
export function formatLogLevel(level: LogLevel): string {
  return level.toUpperCase();
}
