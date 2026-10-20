/**
 * @file JSON 风格值的判定、克隆、冻结与深比较。
 * @description 跨包通用工具；拒绝函数、Date、稀疏数组等不可控输入。
 * @module core/value
 */

import { KurumiError } from './errors.js';

/** JSON 原始值。 */
export type JsonPrimitive = string | number | boolean | null;

/** JSON 风格值。 */
export type JsonValue =
  JsonPrimitive | readonly JsonValue[] | { readonly [key: string]: JsonValue };

/** 最大嵌套深度，防止恶意输入导致栈溢出。 */
export const MAX_JSON_DEPTH = 64;

/** 会污染对象原型的危险键。 */
const FORBIDDEN_KEYS: ReadonlySet<string> = new Set(['__proto__', 'prototype', 'constructor']);

/** 默认路径标签。 */
const DEFAULT_PATH = '$';

/**
 * 判断键是否为原型污染键。
 *
 * @param key - 待判断键
 * @returns 是危险键时返回 true
 */
export function isForbiddenKey(key: string): boolean {
  return FORBIDDEN_KEYS.has(key);
}

/**
 * 判断值是否为普通对象（对象字面量或 `Object.create(null)`）。
 *
 * @param value - 待判断值
 * @returns 是普通对象时返回 true
 */
export function isPlainObject(value: unknown): value is Record<string, unknown> {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return false;
  const prototype: unknown = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

/**
 * 深拷贝一个 JSON 风格值，并拒绝一切非 JSON 值。
 *
 * @param value - 待克隆值
 * @param path - 当前路径标签，用于错误定位
 * @param depth - 内部递归深度；仅供包内递归使用
 * @returns 与入参结构相同的新对象/数组
 * @throws {KurumiError} 出现非 JSON 值、符号键、危险键或超过深度上限
 */
export function cloneJsonValue(
  value: unknown,
  path: string = DEFAULT_PATH,
  depth: number = 0,
): JsonValue {
  assertDepth(depth, path);
  if (value === null || typeof value === 'string' || typeof value === 'boolean') return value;
  if (typeof value === 'number') {
    // JSON 数值语法不允许 NaN / ±Infinity；放行会让输出在 JSON.stringify 时静默变成 null。
    if (!Number.isFinite(value)) {
      throw new KurumiError(`JSON 数值必须是有限数（实际值: ${String(value)}，路径 ${path}）`, {
        code: 'invalid_value',
      });
    }
    return value;
  }
  if (Array.isArray(value)) return cloneJsonArray(value, path, depth);
  if (isPlainObject(value)) return cloneJsonObject(value, path, depth);

  throw new KurumiError(`不支持的 JSON 值类型 ${typeof value}（路径 ${path}）`, {
    code: 'invalid_value',
  });
}

/** 克隆 JSON 数组，拒绝符号键、非下标属性与空槽。 */
function cloneJsonArray(value: readonly unknown[], path: string, depth: number): JsonValue[] {
  if (Object.getOwnPropertySymbols(value).length > 0) {
    throw new KurumiError(`JSON 数组不允许符号键（路径 ${path}）`, { code: 'invalid_value' });
  }
  for (const key of Object.keys(value)) {
    const index = Number(key);
    if (!/^(0|[1-9]\d*)$/.test(key) || index >= value.length) {
      throw new KurumiError(`JSON 数组不允许非下标属性 "${key}"（路径 ${path}）`, {
        code: 'invalid_value',
      });
    }
  }
  const result: JsonValue[] = Array.from({ length: value.length }, () => null);
  for (let index = 0; index < value.length; index += 1) {
    if (!Object.hasOwn(value, index)) {
      throw new KurumiError(`JSON 数组存在空槽（路径 ${childPath(path, index)}）`, {
        code: 'invalid_value',
      });
    }
    result[index] = cloneJsonValue(value[index], childPath(path, index), depth + 1);
  }
  return result;
}

/** 克隆 JSON 普通对象，拒绝符号键与危险键。 */
function cloneJsonObject(
  value: Readonly<Record<string, unknown>>,
  path: string,
  depth: number,
): Record<string, JsonValue> {
  if (Object.getOwnPropertySymbols(value).length > 0) {
    throw new KurumiError(`JSON 对象不允许符号键（路径 ${path}）`, { code: 'invalid_value' });
  }
  const result: Record<string, JsonValue> = {};
  for (const key of Object.keys(value)) {
    if (isForbiddenKey(key)) {
      throw new KurumiError(`JSON 中出现危险键 "${key}"（路径 ${path}）`, {
        code: 'forbidden_key',
      });
    }
    result[key] = cloneJsonValue(value[key], childPath(path, key), depth + 1);
  }
  return result;
}

/** 校验递归深度。 */
function assertDepth(depth: number, path: string): void {
  if (!Number.isSafeInteger(depth) || depth < 0) {
    throw new KurumiError(`内部递归深度必须是非负安全整数（实际值: ${String(depth)}）`, {
      code: 'invalid_depth',
    });
  }
  if (depth > MAX_JSON_DEPTH) {
    throw new KurumiError(`JSON 嵌套深度超过 ${MAX_JSON_DEPTH} 层（路径 ${path}）`, {
      code: 'max_depth_exceeded',
    });
  }
}

/**
 * 递归冻结 JSON 风格值，阻止运行时意外修改。
 *
 * @remarks 迭代遍历，避免深层结构触发调用栈溢出；会直接冻结入参对象。
 * @param value - 待冻结值
 * @returns 原引用，便于链式调用
 *
 * @example
 * ```ts
 * import { deepFreeze } from '@kurumi/core';
 * const value = deepFreeze({ nested: { ok: true } });
 * Object.isFrozen(value.nested); // true
 * ```
 */
export function deepFreeze<TValue extends JsonValue>(value: TValue): TValue;
export function deepFreeze<TValue extends object>(value: TValue): TValue;
export function deepFreeze<TValue>(value: TValue): TValue;
export function deepFreeze(value: unknown): unknown {
  if (typeof value !== 'object' || value === null) return value;

  const seen = new WeakSet<object>();
  const stack: object[] = [value];
  while (stack.length > 0) {
    const current = stack.pop();
    if (current === undefined || seen.has(current)) continue;
    seen.add(current);
    if (Array.isArray(current)) {
      for (const item of current) {
        if (typeof item === 'object' && item !== null) stack.push(item);
      }
    } else {
      for (const key of Object.keys(current)) {
        const item: unknown = Reflect.get(current, key);
        if (typeof item === 'object' && item !== null) stack.push(item);
      }
    }
    Object.freeze(current);
  }
  return value;
}

/**
 * 判断两个 JSON 风格值是否结构相等。
 *
 * @remarks 对象键顺序不影响结果；`Object.is` 语义下 `-0` 与 `0` 不相等。
 * @param left - 左值
 * @param right - 右值
 * @returns 结构完全相同时返回 true
 *
 * @example
 * ```ts
 * import { isDeepEqual } from '@kurumi/core';
 * isDeepEqual({ a: [1] }, { a: [1] }); // true
 * ```
 */
export function isDeepEqual(left: unknown, right: unknown): boolean {
  // 迭代比较而非递归：深层 JSON（例如日志/配置快照）不会触发调用栈溢出。
  const seen = new WeakMap<object, WeakSet<object>>();
  const stack: Array<readonly [unknown, unknown]> = [[left, right]];

  while (stack.length > 0) {
    const pair = stack.pop();
    if (pair === undefined) continue;
    const [currentLeft, currentRight] = pair;
    if (Object.is(currentLeft, currentRight)) continue;
    if (
      typeof currentLeft !== 'object' ||
      currentLeft === null ||
      typeof currentRight !== 'object' ||
      currentRight === null
    ) {
      return false;
    }

    // 同一对象对只比较一次：既避免循环引用无限递归，也保持共享引用的结构语义。
    const seenRight = seen.get(currentLeft);
    if (seenRight?.has(currentRight) === true) continue;
    if (seenRight === undefined) {
      seen.set(currentLeft, new WeakSet([currentRight]));
    } else {
      seenRight.add(currentRight);
    }

    if (Array.isArray(currentLeft) || Array.isArray(currentRight)) {
      if (
        !Array.isArray(currentLeft) ||
        !Array.isArray(currentRight) ||
        currentLeft.length !== currentRight.length
      ) {
        return false;
      }
      for (let index = currentLeft.length - 1; index >= 0; index -= 1) {
        stack.push([currentLeft[index], currentRight[index]]);
      }
      continue;
    }

    if (!isPlainObject(currentLeft) || !isPlainObject(currentRight)) return false;
    const leftKeys = Object.keys(currentLeft);
    const rightKeys = Object.keys(currentRight);
    if (leftKeys.length !== rightKeys.length) return false;
    for (const key of leftKeys) {
      if (!Object.hasOwn(currentRight, key)) return false;
      stack.push([currentLeft[key], currentRight[key]]);
    }
  }

  return true;
}

/** 在路径标签后追加键或下标。 */
function childPath(base: string, key: string | number): string {
  return typeof key === 'number' ? `${base}[${key}]` : `${base}.${key}`;
}
