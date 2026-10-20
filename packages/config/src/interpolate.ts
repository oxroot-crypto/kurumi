/**
 * @file 配置字符串插值：`${VAR}`、`${VAR:-fallback}` 与 `{env:VAR}`。
 * @description 插值只作用于字符串值，不作用于键；缺失变量默认报错，可通过 shouldAllowMissing 保留原文。
 * @module config/interpolate
 */

import { MAX_JSON_DEPTH, isForbiddenKey, isPlainObject } from '@kurumi/core';

import { ConfigError, ConfigInterpolationError } from './errors.js';
import { ConfigIssueCode, childPath, createIssue, type ConfigIssue } from './issue.js';
import type { ConfigLayer } from './source.js';

/** 合法环境变量名语法。 */
const ENV_NAME_PATTERN = /^[A-Za-z_][A-Za-z0-9_]*$/;

/**
 * 统一占位符：`$${NAME}` 转义、`${NAME}` 与 `{env:NAME}`。
 * @remarks 使用单次正则扫描，避免前缀匹配把普通文本误判为占位符。
 */
const PLACEHOLDER_PATTERN = /\$\$\{([^}]*)\}|\$\{([^}]*)\}|\{env:([^}]*)\}/g;

/** 插值选项。 */
export interface InterpolationOptions {
  /**
   * 缺失变量时是否保留原始占位符。
   *
   * @default false
   */
  readonly shouldAllowMissing?: boolean;
}

interface InterpolationContext {
  readonly env: Readonly<Record<string, string | undefined>>;
  readonly options: InterpolationOptions;
  readonly issues: ConfigIssue[];
}

/**
 * 对整层配置执行递归字符串插值。
 *
 * @remarks
 * - 支持 `${NAME}`、`${NAME:-默认值}`、`{env:NAME}` 与 `{env:NAME:-默认值}`。
 * - 环境变量值为空字符串时视为“已定义”。
 * - `$${NAME}` 转义为字面量 `${NAME}`。
 * - 缺失变量且无默认值时，默认抛出聚合错误。
 *
 * @param layer - 待插值配置层
 * @param env - 环境变量快照
 * @param options - 插值选项
 * @returns 新的配置层，不修改入参
 * @throws {ConfigInterpolationError} 存在无法解析的占位符
 * @throws {ConfigError} 配置嵌套超过上限
 *
 * @example
 * ```ts
 * interpolateConfig(
 *   { url: '${HOST:-localhost}:{env:PORT}' },
 *   { PORT: '3000' },
 * ); // { url: 'localhost:3000' }
 * ```
 */
export function interpolateConfig(
  layer: ConfigLayer,
  env: Readonly<Record<string, string | undefined>>,
  options: InterpolationOptions = {},
): ConfigLayer {
  if (typeof options !== 'object' || options === null || Array.isArray(options)) {
    throw new ConfigError('InterpolationOptions 必须是普通对象', { code: 'invalid_options' });
  }
  if (options.shouldAllowMissing !== undefined && typeof options.shouldAllowMissing !== 'boolean') {
    throw new ConfigError(
      `插值选项 shouldAllowMissing 必须是布尔值（实际值: ${String(options.shouldAllowMissing)}）`,
      { code: 'invalid_options' },
    );
  }
  const context: InterpolationContext = { env, options, issues: [] };
  const result = interpolateValue(layer, '$', 0, context);
  if (context.issues.length > 0) throw new ConfigInterpolationError(context.issues);
  if (!isPlainObject(result)) {
    throw new ConfigError('插值结果必须是普通对象', { code: 'invalid_layer' });
  }
  return result;
}

function interpolateValue(
  value: unknown,
  path: string,
  depth: number,
  context: InterpolationContext,
): unknown {
  if (depth > MAX_JSON_DEPTH) {
    throw new ConfigError(`配置插值嵌套深度超过 ${MAX_JSON_DEPTH} 层（路径 ${path}）`, {
      code: 'max_depth_exceeded',
    });
  }
  if (typeof value === 'string') return interpolateString(value, path, context);
  if (Array.isArray(value)) {
    return value.map((item, index) =>
      interpolateValue(item, childPath(path, index), depth + 1, context),
    );
  }
  if (isPlainObject(value)) {
    const result: Record<string, unknown> = {};
    for (const key of Object.keys(value)) {
      if (isForbiddenKey(key)) {
        throw new ConfigInterpolationError([
          createIssue(ConfigIssueCode.ForbiddenKey, path, `配置插值遇到危险键 "${key}"`),
        ]);
      }
      result[key] = interpolateValue(value[key], childPath(path, key), depth + 1, context);
    }
    return result;
  }
  return value;
}

function interpolateString(input: string, path: string, context: InterpolationContext): string {
  let result = '';
  let lastIndex = 0;

  // 每次调用都重置 lastIndex，避免共享正则的全局状态跨字符串泄漏。
  PLACEHOLDER_PATTERN.lastIndex = 0;
  let match = PLACEHOLDER_PATTERN.exec(input);
  while (match !== null) {
    result += input.slice(lastIndex, match.index);
    const escaped = match[1];
    if (escaped !== undefined) {
      // `$${NAME}` 是转义写法，输出字面量 `${NAME}` 且不再解析。
      result += `\${${escaped}}`;
    } else {
      const expression = match[2] ?? match[3] ?? '';
      result += resolvePlaceholder(expression, match[0], path, context);
    }
    lastIndex = match.index + match[0].length;
    match = PLACEHOLDER_PATTERN.exec(input);
  }
  result += input.slice(lastIndex);
  return result;
}

function resolvePlaceholder(
  expression: string,
  placeholder: string,
  path: string,
  context: InterpolationContext,
): string {
  const separatorIndex = expression.indexOf(':-');
  const name = separatorIndex < 0 ? expression : expression.slice(0, separatorIndex);
  const fallback = separatorIndex < 0 ? undefined : expression.slice(separatorIndex + 2);
  return resolveEnvValue(name, fallback, placeholder, path, context);
}

/** 读取环境变量；缺失时按 fallback、shouldAllowMissing 或错误处理。 */
function resolveEnvValue(
  name: string,
  fallback: string | undefined,
  placeholder: string,
  path: string,
  context: InterpolationContext,
): string {
  if (!ENV_NAME_PATTERN.test(name)) return placeholder;

  const envValue = context.env[name];
  if (envValue !== undefined) return envValue;
  if (fallback !== undefined) return fallback;
  if (context.options.shouldAllowMissing === true) return placeholder;

  context.issues.push(
    createIssue(ConfigIssueCode.MissingEnvVar, path, `环境变量 "${name}" 未定义，且未提供默认值`, {
      expected: `environment variable ${name}`,
      received: 'undefined',
    }),
  );
  return placeholder;
}
