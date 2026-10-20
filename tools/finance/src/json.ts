/**
 * @file Agent 工具结果与 JSON 安全转换。
 * @description Decimal/BigInt 转字符串，递归处理数组/对象并丢弃危险键；输出满足 `ToolResultInput.metadata`。
 * @module finance-tools/json
 */

import { isForbiddenKey, isPlainObject, type JsonValue } from '@kurumi/core';
import type { ToolResultInput } from '@kurumi/agent';
import { Decimal } from '@kurumi/math';

/**
 * 把任意领域值转换为 JSON 安全值。
 *
 * @param value - 领域值
 * @returns JSON 安全值；`undefined` 返回 undefined
 */
export function toJsonValue(value: unknown): JsonValue | undefined {
  if (value === undefined) return undefined;
  if (value === null || typeof value === 'string' || typeof value === 'boolean') {
    return value;
  }
  if (typeof value === 'number') {
    return Number.isFinite(value) ? value : String(value);
  }
  if (typeof value === 'bigint') return value.toString();
  if (value instanceof Decimal) return value.toString();
  if (Array.isArray(value)) {
    return value.map((item) => toJsonValue(item) ?? null);
  }
  if (isPlainObject(value)) {
    const result: Record<string, JsonValue> = {};
    for (const [key, item] of Object.entries(value)) {
      if (isForbiddenKey(key)) continue;
      const converted = toJsonValue(item);
      if (converted !== undefined) result[key] = converted;
    }
    return result;
  }
  return String(value);
}

/**
 * 构造 Agent 工具结果。
 *
 * @param value - 领域值
 * @param isError - 是否标记为错误
 * @returns `content` 为 JSON 文本，`metadata` 为同一 JSON 安全值
 */
export function toolResult(value: unknown, isError = false): ToolResultInput {
  const json = toJsonValue(value) ?? null;
  return {
    content: JSON.stringify(json),
    isError,
    metadata: json,
  };
}
