/**
 * @file 严格 JSON 解析。
 * @description 在 `JSON.parse` 之上补充 BOM 处理、来源上下文与统一语法错误类型。
 * @module config/parser/json
 */

import { ConfigSyntaxError } from '../errors.js';

/** 从 JSON 错误消息中提取字符偏移。 */
function extractOffset(message: string): number {
  const matched = /position (\d+)/i.exec(message);
  if (matched?.[1] === undefined) return 0;
  const parsed = Number.parseInt(matched[1], 10);
  return Number.isSafeInteger(parsed) ? parsed : 0;
}

/**
 * 解析严格 JSON 文本。
 *
 * @param text - 原始文本，可带 UTF-8 BOM
 * @param sourceName - 用于错误信息的来源名称
 * @returns 解析后的任意 JSON 值
 * @throws {ConfigSyntaxError} 文本不是合法 JSON
 */
export function parseJson(text: string, sourceName: string): unknown {
  const normalized = text.charCodeAt(0) === 0xfeff ? text.slice(1) : text;
  try {
    return JSON.parse(normalized);
  } catch (cause) {
    const message = cause instanceof Error ? cause.message : String(cause);
    throw new ConfigSyntaxError(sourceName, message, extractOffset(message), cause);
  }
}
