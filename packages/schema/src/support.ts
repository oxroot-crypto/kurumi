/**
 * @file schema 实现共享的小工具：必填/类型错误、选项校验、正则归一化与深度防护。
 * @description 仅服务于 schema 层，不对外导出。
 * @module schema/support
 */

import { MAX_JSON_DEPTH, isForbiddenKey, isPlainObject } from '@kurumi/core';

import { SchemaDefinitionError } from './errors.js';
import { SchemaIssueCode, childPath, createIssue, type SchemaIssue } from './issue.js';
import { Schema, parseFailure, type ParseFailure, type SchemaMetadata } from './schema.js';

/** 构造默认元数据。 */
export function createMetadata(): SchemaMetadata {
  return { isSecret: false };
}

/** 校验定义期选项必须是普通对象，避免 null/数组导致原生 TypeError。 */
export function assertOptionsObject(name: string, value: unknown): void {
  if (!isPlainObject(value)) {
    throw new SchemaDefinitionError(`${name} 必须是普通对象（实际值: ${String(value)}）`);
  }
}

/** 校验子 schema，保证组合 schema 在构造期就能拒绝非法结构。 */
export function assertSchema(name: string, value: unknown): asserts value is Schema<unknown> {
  if (!(value instanceof Schema)) {
    throw new SchemaDefinitionError(`${name} 必须是 Schema 实例（实际值: ${String(value)}）`);
  }
}

/** 校验对象 shape：必须是普通对象，且每个值都是 Schema。 */
export function assertSchemaShape(
  name: string,
  value: unknown,
): asserts value is Readonly<Record<string, Schema<unknown>>> {
  if (!isPlainObject(value)) {
    throw new SchemaDefinitionError(`${name} 必须是普通对象（实际值: ${String(value)}）`);
  }
  for (const [key, childSchema] of Object.entries(value)) {
    if (!(childSchema instanceof Schema)) {
      throw new SchemaDefinitionError(`${name}["${key}"] 必须是 Schema 实例`);
    }
  }
}

/** 校验可选字符串选项。 */
export function assertOptionalString(name: string, value: unknown): void {
  if (value !== undefined && typeof value !== 'string') {
    throw new SchemaDefinitionError(`${name} 必须是字符串（实际值: ${String(value)}）`);
  }
}

/** 构造“字段缺失”失败结果。 */
export function requiredFailure(path: string, expected: string): ParseFailure {
  return {
    ok: false,
    issues: [
      createIssue(SchemaIssueCode.Required, path, '缺少必填字段（实际值: undefined）', {
        expected,
        received: 'undefined',
      }),
    ],
  };
}

/** 构造“类型不符”失败结果。 */
export function invalidTypeFailure(path: string, expected: string, value: unknown): ParseFailure {
  return {
    ok: false,
    issues: [
      createIssue(
        SchemaIssueCode.InvalidType,
        path,
        `期望 ${expected}，实际为 ${describeValue(value)}`,
        {
          expected,
          received: describeValue(value),
        },
      ),
    ],
  };
}

/** 比 `typeof` 更精确的实际值描述（用于错误信息）。 */
export function describeValue(value: unknown): string {
  if (value === null) return 'null';
  if (Array.isArray(value)) return 'array';
  if (typeof value === 'number' && Number.isNaN(value)) return 'NaN';
  return typeof value;
}

/** 校验非负整数选项。 */
export function assertNonNegativeIntegerOption(name: string, value: number | undefined): void {
  if (value === undefined) return;
  if (!Number.isInteger(value) || value < 0) {
    throw new SchemaDefinitionError(`${name} 必须是非负整数，实际为 ${String(value)}`);
  }
}

/** 校验有限数值选项。 */
export function assertFiniteNumberOption(name: string, value: number | undefined): void {
  if (value === undefined) return;
  if (!Number.isFinite(value)) {
    throw new SchemaDefinitionError(`${name} 必须是有限数值，实际为 ${String(value)}`);
  }
}

/** 校验下界不大于上界。 */
export function assertOrderedBounds(
  name: string,
  min: number | undefined,
  max: number | undefined,
): void {
  if (min !== undefined && max !== undefined && min > max) {
    throw new SchemaDefinitionError(`${name} 下限 ${min} 不能大于上限 ${max}`);
  }
}

/**
 * 克隆正则并移除 `g`/`y` 标志，避免 `lastIndex` 状态导致校验结果漂移。
 */
export function normalizePattern(pattern: RegExp): RegExp {
  return new RegExp(pattern.source, pattern.flags.replace(/[gy]/g, ''));
}

/** 按 Unicode 码点计算字符串长度（emoji 计为 1）。 */
export function codePointLength(value: string): number {
  return [...value].length;
}

/** 校验嵌套深度，超限时返回失败结果。 */
export function assertDepth(depth: number, path: string): ParseFailure | undefined {
  if (depth <= MAX_JSON_DEPTH) return undefined;
  return parseFailure([
    createIssue(SchemaIssueCode.MaxDepth, path, `schema 嵌套深度超过上限 ${MAX_JSON_DEPTH}`, {
      expected: `depth <= ${MAX_JSON_DEPTH}`,
      received: `depth ${depth}`,
    }),
  ]);
}

/** 生成危险键问题列表。 */
export function forbiddenKeyIssues(keys: readonly string[], path: string): SchemaIssue[] {
  return keys
    .filter((key) => isForbiddenKey(key))
    .map((key) =>
      createIssue(SchemaIssueCode.ForbiddenKey, childPath(path, key), `禁止使用危险键 "${key}"`),
    );
}

/** 校验集合长度/键数量边界。 */
export function sizeIssues(
  length: number,
  path: string,
  label: string,
  bounds: { readonly min: number | undefined; readonly max: number | undefined },
): SchemaIssue[] {
  const issues: SchemaIssue[] = [];
  if (bounds.min !== undefined && length < bounds.min) {
    issues.push(
      createIssue(SchemaIssueCode.TooSmall, path, `${label} ${length} 小于下限 ${bounds.min}`, {
        expected: `>= ${bounds.min}`,
        received: String(length),
      }),
    );
  }
  if (bounds.max !== undefined && length > bounds.max) {
    issues.push(
      createIssue(SchemaIssueCode.TooLarge, path, `${label} ${length} 大于上限 ${bounds.max}`, {
        expected: `<= ${bounds.max}`,
        received: String(length),
      }),
    );
  }
  return issues;
}

/**
 * 判断一组失败是否比另一组更具体：路径更深优先，其次问题更少。
 */
export function isMoreSpecificFailure(
  candidate: readonly SchemaIssue[],
  current: readonly SchemaIssue[],
): boolean {
  const candidateDepth = maxPathLength(candidate);
  const currentDepth = maxPathLength(current);
  if (candidateDepth !== currentDepth) return candidateDepth > currentDepth;
  return candidate.length < current.length;
}

/** 计算 issue 路径的最大长度。 */
export function maxPathLength(issues: readonly SchemaIssue[]): number {
  let maximum = 0;
  for (const issue of issues) maximum = Math.max(maximum, issue.path.length);
  return maximum;
}
