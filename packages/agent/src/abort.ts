/**
 * @file AbortSignal 判定工具。
 * @description 通过函数调用读取状态，避免 TypeScript 对只读属性的控制流窄化影响后续检查。
 * @module agent/abort
 */

/**
 * 判断取消信号是否已触发。
 *
 * @param signal - 取消信号；undefined 视为未取消
 * @returns 已触发时返回 true
 */
export function isAborted(signal: AbortSignal | undefined): boolean {
  return signal?.aborted === true;
}
