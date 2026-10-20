/**
 * @file 取消信号与取消错误的跨 runtime 判定工具。
 * @description 使用结构化判断，兼容 DOMException、Node ABORT_ERR 与跨 realm AbortSignal。
 * @module core/abort
 */

/**
 * 判断值是否具备 AbortSignal 形状。
 *
 * @param value - 待判断值
 * @returns 具备 aborted/addEventListener/removeEventListener 时返回 true
 */
export function isAbortSignal(value: unknown): value is AbortSignal {
  if (typeof value !== 'object' || value === null) return false;
  return (
    typeof Reflect.get(value, 'aborted') === 'boolean' &&
    typeof Reflect.get(value, 'addEventListener') === 'function' &&
    typeof Reflect.get(value, 'removeEventListener') === 'function'
  );
}

/**
 * 判断值是否表示取消错误。
 *
 * @param value - 待判断值
 * @returns `name === 'AbortError'` 或 `code === 'ABORT_ERR'` 时返回 true
 */
export function isAbortError(value: unknown): boolean {
  if (typeof value !== 'object' || value === null) return false;
  try {
    return (
      Reflect.get(value, 'name') === 'AbortError' || Reflect.get(value, 'code') === 'ABORT_ERR'
    );
  } catch {
    return false;
  }
}
