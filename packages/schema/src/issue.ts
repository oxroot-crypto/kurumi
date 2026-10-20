/**
 * @file 通用 schema 问题（SchemaIssue）的编码、路径与展示工具。
 * @description 所有 schema 校验失败都复用同一套结构化 issue，便于聚合、展示与上层改码。
 * @module schema/issue
 */

/** schema 树根路径的展示形式。 */
export const ROOT_PATH = '$';

/** 安全占位符：被标记为 secret 的值在序列化时替换成该字符串。 */
export const REDACTED_VALUE = '[REDACTED]';

/** 通用 schema 问题的错误码集合。 */
export const SchemaIssueCode = Object.freeze({
  /** 必填字段缺失（值为 undefined）。 */
  Required: 'required',
  /** 值的基础类型不符合 schema。 */
  InvalidType: 'invalid_type',
  /** 对象中出现 schema 未声明的键。 */
  UnknownKey: 'unknown_key',
  /** 出现会污染原型的危险键。 */
  ForbiddenKey: 'forbidden_key',
  /** 字符串/数组长度或数值低于下界。 */
  TooSmall: 'too_small',
  /** 字符串/数组长度或数值高于上界。 */
  TooLarge: 'too_large',
  /** 字符串不匹配正则。 */
  PatternMismatch: 'pattern_mismatch',
  /** 数值不是整数。 */
  NotInteger: 'not_integer',
  /** 数值为 NaN 或无穷。 */
  NotFinite: 'not_finite',
  /** 字面量不匹配。 */
  InvalidLiteral: 'invalid_literal',
  /** 联合类型的所有分支均不匹配。 */
  UnionMismatch: 'union_mismatch',
  /** 值不是合法的 JSON 风格数据（函数、Date、稀疏数组等）。 */
  InvalidValue: 'invalid_value',
  /** 嵌套超过深度上限。 */
  MaxDepth: 'max_depth_exceeded',
} as const);

/** 通用 schema 问题错误码联合类型。 */
export type SchemaIssueCode = (typeof SchemaIssueCode)[keyof typeof SchemaIssueCode];

/** 一条可定位的 schema 问题。 */
export interface SchemaIssue {
  /**
   * 错误码，供程序分支处理。
   * @remarks 使用 `string` 以允许上层域（如 config）追加专用错误码。
   */
  readonly code: string;
  /** 数据路径，例如 `$.server.port` 或 `$.hosts[0]`。 */
  readonly path: string;
  /** 面向人类的可读描述，必须包含实际值/期望值等定位信息。 */
  readonly message: string;
  /** 期望的类型或约束描述。 */
  readonly expected?: string;
  /** 实际值的简短类型描述。 */
  readonly received?: string;
}

/**
 * 在父路径后追加一个对象键或数组下标。
 *
 * @param base - 父路径，根路径为 `$`
 * @param key - 对象键或数组下标
 * @returns 新的路径字符串
 *
 * @example
 * ```ts
 * childPath('$.server', 'port'); // '$.server.port'
 * childPath('$.hosts', 0);       // '$.hosts[0]'
 * ```
 */
export function childPath(base: string, key: string | number): string {
  return typeof key === 'number' ? `${base}[${key}]` : `${base}.${key}`;
}

/**
 * 将根路径转换为适合展示的名称。
 *
 * @param path - 数据路径
 * @returns 根路径显示为 `<root>`，其他路径原样返回
 */
export function displayPath(path: string): string {
  return path === ROOT_PATH ? '<root>' : path;
}

/**
 * 返回值的简短类型描述，用于错误信息。
 *
 * @param value - 待描述值
 * @returns `null`、`array` 或 `typeof` 结果
 */
export function describeType(value: unknown): string {
  if (value === null) return 'null';
  if (Array.isArray(value)) return 'array';
  return typeof value;
}

/**
 * 构造一条 schema 问题，自动补齐可选字段。
 *
 * @param code - 错误码
 * @param path - 数据路径
 * @param message - 可读描述
 * @param details - 可选的期望值/实际值描述
 * @returns 不可变的 SchemaIssue 对象
 */
export function createIssue(
  code: SchemaIssueCode,
  path: string,
  message: string,
  details?: { readonly expected?: string; readonly received?: string },
): SchemaIssue {
  return Object.freeze({
    code,
    path,
    message,
    ...(details?.expected === undefined ? {} : { expected: details.expected }),
    ...(details?.received === undefined ? {} : { received: details.received }),
  });
}

/**
 * 将 issue 列表渲染为多行错误文本。
 *
 * @param issues - schema 问题列表
 * @returns 每行一条、带路径前缀的文本
 */
export function formatIssues(issues: readonly SchemaIssue[]): string {
  return issues.map((issue) => `  - ${displayPath(issue.path)}: ${issue.message}`).join('\n');
}
