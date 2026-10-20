/**
 * @file 日志记录类型与字段归一化。
 * @description 所有进入 sink 的字段必须已转换为 JSON 安全值并递归冻结。
 * @module log/record
 */

import {
  deepFreeze,
  isForbiddenKey,
  isPlainObject,
  toJsonSafeValue,
  type JsonValue,
} from '@kurumi/core';

import type { EmittedLogLevel } from './level.js';

/** 可安全写入 sink 的日志值。 */
export type LogValue = JsonValue;

/** 调用方传入的结构化字段。 */
export type LogFields = Readonly<Record<string, unknown>>;

/** 归一化后的字段或 bindings。 */
export type ResolvedLogFields = Readonly<Record<string, LogValue>>;

/** 一条不可变日志记录。 */
export interface LogRecord {
  /** 事件时间戳（毫秒，与 `Date.now()` 同基准）。 */
  readonly timeMs: number;
  /** 写入级别。 */
  readonly level: EmittedLogLevel;
  /** 人类可读消息。 */
  readonly message: string;
  /** 由 `child()` 累积的绑定字段。 */
  readonly bindings: ResolvedLogFields;
  /** 本条日志的字段；没有字段时省略。 */
  readonly fields?: ResolvedLogFields;
}

/** 字段归一化选项。 */
export interface RecordNormalizationOptions {
  /** 最大递归深度。 */
  readonly maxDepth: number;
  /** 字符串最大长度。 */
  readonly maxStringLength: number;
  /** 集合最大条目数。 */
  readonly maxEntries: number;
  /** 是否保留 Error 调用栈。 */
  readonly shouldIncludeStack: boolean;
}

/** 空 bindings 的共享冻结实例。 */
export const EMPTY_LOG_FIELDS: ResolvedLogFields = deepFreeze({});

/**
 * 将调用方字段归一化为 JSON 安全的冻结对象。
 *
 * @remarks
 * - `undefined` / `null` 返回空对象。
 * - 非普通对象（数组、Map、类实例等）包装为 `{ value: ... }`，避免丢失诊断信息。
 * - `undefined` 字段被忽略，危险键被丢弃，顶层用户字段数量受 `maxEntries` 限制；
 *   超过上限时额外增加一个 `[truncated]` 元数据键。
 *
 * @param value - 原始字段
 * @param options - 序列化限制
 * @returns 冻结的字段对象
 */
export function normalizeLogFields(
  value: unknown,
  options: RecordNormalizationOptions,
): ResolvedLogFields {
  if (value === undefined || value === null) return EMPTY_LOG_FIELDS;
  if (!isPlainObject(value)) {
    return deepFreeze({ value: toJsonSafeValue(value, options) });
  }

  const keys = Object.keys(value);
  const limit = Math.min(keys.length, options.maxEntries);
  const result: Record<string, LogValue> = {};
  for (let index = 0; index < limit; index += 1) {
    const key = keys[index];
    if (key === undefined || isForbiddenKey(key)) continue;
    const field = readField(value, key);
    if (field === undefined) continue;
    const safeKey = toJsonSafeValue(key, options);
    const keyText = typeof safeKey === 'string' ? safeKey : key;
    result[keyText] = toJsonSafeValue(field, options);
  }
  if (keys.length > limit) {
    result['[truncated]'] = `[${keys.length - limit} more keys]`;
  }
  if (Object.keys(result).length === 0) return EMPTY_LOG_FIELDS;
  return deepFreeze(result);
}

/**
 * 浅合并两组已归一化字段，后者覆盖前者。
 *
 * @param base - 父级 bindings
 * @param override - 子级 bindings
 * @returns 新的冻结对象，不修改入参
 */
export function mergeLogFields(
  base: ResolvedLogFields,
  override: ResolvedLogFields,
): ResolvedLogFields {
  return deepFreeze({ ...base, ...override });
}

function readField(value: Readonly<Record<string, unknown>>, key: string): unknown {
  try {
    return value[key];
  } catch {
    return '[Unreadable]';
  }
}
