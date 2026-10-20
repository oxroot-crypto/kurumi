/**
 * @file 配置问题（ConfigIssue）的编码、路径与类型描述工具。
 * @description 复用 `@kurumi/schema` 的通用错误码与展示工具，仅追加配置来源/插值专用错误码。
 * @module config/issue
 */

import {
  REDACTED_VALUE,
  ROOT_PATH,
  SchemaIssueCode,
  childPath,
  describeType,
  displayPath,
  formatIssues,
  type SchemaIssue,
} from '@kurumi/schema';

export { REDACTED_VALUE, ROOT_PATH, childPath, describeType, displayPath, formatIssues };

/** 配置问题错误码集合：通用 schema 码 + 配置流水线专用码。 */
export const ConfigIssueCode = Object.freeze({
  ...SchemaIssueCode,
  /** 插值引用的环境变量缺失且没有默认值。 */
  MissingEnvVar: 'missing_env_var',
  /** 环境变量映射路径冲突（例如同时当作对象与数组）。 */
  EnvConflict: 'env_conflict',
  /** 环境变量数组索引不连续。 */
  EnvIndexGap: 'env_index_gap',
  /** 配置文件根节点不是对象。 */
  InvalidRoot: 'invalid_root',
} as const);

/** 结构化配置问题的错误码联合类型。 */
export type ConfigIssueCode = (typeof ConfigIssueCode)[keyof typeof ConfigIssueCode];

/** 一条可定位的配置问题。 */
export interface ConfigIssue extends SchemaIssue {
  /** 错误码，供程序分支处理。 */
  readonly code: ConfigIssueCode;
}

/**
 * 构造一条配置问题，自动补齐可选字段。
 *
 * @param code - 错误码
 * @param path - 配置树路径
 * @param message - 可读描述
 * @param details - 可选的期望值/实际值描述
 * @returns 不可变的 ConfigIssue 对象
 */
export function createIssue(
  code: ConfigIssueCode,
  path: string,
  message: string,
  details?: { readonly expected?: string; readonly received?: string },
): ConfigIssue {
  return Object.freeze({
    code,
    path,
    message,
    ...(details?.expected === undefined ? {} : { expected: details.expected }),
    ...(details?.received === undefined ? {} : { received: details.received }),
  });
}
