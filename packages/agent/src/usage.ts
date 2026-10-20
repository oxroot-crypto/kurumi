/**
 * @file Token 用量计数、归一化与累加。
 * @description 所有计数均为非负安全整数，`totalTokens` 始终等于输入与输出之和。
 * @module agent/usage
 */

import { isPlainObject } from '@kurumi/core';

import { AgentDefinitionError } from './errors.js';

/** 一次模型调用的 Token 用量。 */
export interface TokenUsage {
  /** 输入 Token 数，非负安全整数。 */
  readonly inputTokens: number;
  /** 输出 Token 数，非负安全整数。 */
  readonly outputTokens: number;
  /** 总 Token 数，恒等于 `inputTokens + outputTokens`。 */
  readonly totalTokens: number;
  /** 命中缓存的输入 Token 数（可选）。 */
  readonly cacheReadTokens?: number;
  /** 写入缓存的输入 Token 数（可选）。 */
  readonly cacheWriteTokens?: number;
}

/** 零用量，供未提供 usage 的模型适配器复用。 */
export const EMPTY_TOKEN_USAGE: TokenUsage = Object.freeze({
  inputTokens: 0,
  outputTokens: 0,
  totalTokens: 0,
});

/** `createTokenUsage` 的可选缓存计数。 */
export interface TokenUsageOptions {
  /** 命中缓存的输入 Token 数。 */
  readonly cacheReadTokens?: number;
  /** 写入缓存的输入 Token 数。 */
  readonly cacheWriteTokens?: number;
}

/**
 * 根据输入/输出 Token 数构造冻结的用量对象。
 *
 * @param inputTokens - 输入 Token 数，非负安全整数
 * @param outputTokens - 输出 Token 数，非负安全整数
 * @param options - 可选缓存计数，均为非负安全整数
 * @returns 冻结的用量对象，`totalTokens` 自动计算
 * @throws {AgentDefinitionError} 任一计数非安全整数、为负数或总和溢出
 *
 * @example
 * ```ts
 * createTokenUsage(100, 20);
 * // { inputTokens: 100, outputTokens: 20, totalTokens: 120 }
 * ```
 */
export function createTokenUsage(
  inputTokens: number,
  outputTokens: number,
  options?: TokenUsageOptions,
): TokenUsage {
  assertCount(inputTokens, 'inputTokens');
  assertCount(outputTokens, 'outputTokens');
  const totalTokens = inputTokens + outputTokens;
  assertCount(totalTokens, 'totalTokens');
  const cacheReadTokens = options?.cacheReadTokens;
  const cacheWriteTokens = options?.cacheWriteTokens;
  if (cacheReadTokens !== undefined) assertCount(cacheReadTokens, 'cacheReadTokens');
  if (cacheWriteTokens !== undefined) assertCount(cacheWriteTokens, 'cacheWriteTokens');

  return Object.freeze({
    inputTokens,
    outputTokens,
    totalTokens,
    ...(cacheReadTokens === undefined ? {} : { cacheReadTokens }),
    ...(cacheWriteTokens === undefined ? {} : { cacheWriteTokens }),
  });
}

/**
 * 校验并归一化来自模型适配器的 usage 数据。
 *
 * @param value - 未经信任的 usage 值
 * @param path - 出错路径，用于诊断
 * @returns 冻结的用量对象；缺省 `totalTokens` 时自动计算
 * @throws {AgentDefinitionError} 结构非法、计数越界或总数与分项不一致
 */
export function normalizeTokenUsage(value: unknown, path = '$'): TokenUsage {
  if (!isPlainObject(value)) {
    throw new AgentDefinitionError(`${path} 必须是普通对象`);
  }
  const inputTokens = value['inputTokens'];
  const outputTokens = value['outputTokens'];
  assertCount(inputTokens, `${path}.inputTokens`);
  assertCount(outputTokens, `${path}.outputTokens`);

  const totalTokens = value['totalTokens'];
  if (totalTokens !== undefined) {
    assertCount(totalTokens, `${path}.totalTokens`);
    if (totalTokens !== inputTokens + outputTokens) {
      throw new AgentDefinitionError(
        `${path}.totalTokens 必须等于 inputTokens + outputTokens（期望 ${inputTokens + outputTokens}，实际 ${totalTokens}）`,
      );
    }
  }

  const cacheReadTokens = value['cacheReadTokens'];
  const cacheWriteTokens = value['cacheWriteTokens'];
  if (cacheReadTokens !== undefined) assertCount(cacheReadTokens, `${path}.cacheReadTokens`);
  if (cacheWriteTokens !== undefined) assertCount(cacheWriteTokens, `${path}.cacheWriteTokens`);

  return createTokenUsage(inputTokens, outputTokens, {
    ...(cacheReadTokens === undefined ? {} : { cacheReadTokens }),
    ...(cacheWriteTokens === undefined ? {} : { cacheWriteTokens }),
  });
}

/**
 * 累加两次模型调用的用量。
 *
 * @param left - 左值用量
 * @param right - 右值用量
 * @returns 新的冻结用量对象；缓存计数在任一侧存在时按 0 补齐后相加
 * @throws {AgentDefinitionError} 累加结果溢出安全整数
 */
export function addTokenUsage(left: TokenUsage, right: TokenUsage): TokenUsage {
  const cacheReadTokens =
    left.cacheReadTokens === undefined && right.cacheReadTokens === undefined
      ? undefined
      : (left.cacheReadTokens ?? 0) + (right.cacheReadTokens ?? 0);
  const cacheWriteTokens =
    left.cacheWriteTokens === undefined && right.cacheWriteTokens === undefined
      ? undefined
      : (left.cacheWriteTokens ?? 0) + (right.cacheWriteTokens ?? 0);

  return createTokenUsage(
    left.inputTokens + right.inputTokens,
    left.outputTokens + right.outputTokens,
    {
      ...(cacheReadTokens === undefined ? {} : { cacheReadTokens }),
      ...(cacheWriteTokens === undefined ? {} : { cacheWriteTokens }),
    },
  );
}

/** 校验非负安全整数。 */
function assertCount(value: unknown, path: string): asserts value is number {
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0) {
    throw new AgentDefinitionError(`${path} 必须是非负安全整数（实际值: ${String(value)}）`);
  }
}
