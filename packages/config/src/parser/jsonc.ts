/**
 * @file JSONC（带注释与尾随逗号）解析。
 * @description 先做尊重字符串的注释/尾随逗号清理，再交给严格 JSON 解析器。
 * @module config/parser/jsonc
 */

import { ConfigSyntaxError } from '../errors.js';

/**
 * 解析 JSONC 文本。
 *
 * @remarks
 * 支持 `//` 行注释、`/* *\/` 块注释、对象/数组尾随逗号与 UTF-8 BOM。
 * 注释内容不会进入输出；字符串内的注释符号保持原样。
 *
 * @param text - 原始 JSONC 文本
 * @param sourceName - 用于错误信息的来源名称
 * @returns 解析后的任意 JSON 值
 * @throws {ConfigSyntaxError} 注释未闭合、字符串未闭合或 JSON 语法非法
 */
export function parseJsonc(text: string, sourceName: string): unknown {
  const normalized = text.charCodeAt(0) === 0xfeff ? text.slice(1) : text;
  const cleaned = stripCommentsAndTrailingCommas(normalized, sourceName);
  try {
    return JSON.parse(cleaned);
  } catch (cause) {
    const message = cause instanceof Error ? cause.message : String(cause);
    const matched = /position (\d+)/i.exec(message);
    const offset = matched?.[1] === undefined ? 0 : Number.parseInt(matched[1], 10);
    throw new ConfigSyntaxError(
      sourceName,
      message,
      Number.isSafeInteger(offset) ? offset : 0,
      cause,
    );
  }
}

interface StringScan {
  readonly index: number;
  readonly isInString: boolean;
}

interface BlockCommentScan {
  readonly index: number;
  readonly newlines: readonly string[];
}

/**
 * 清理注释与尾随逗号，同时保留字符串内容与换行位置。
 *
 * @param text - 已去除 BOM 的文本
 * @param sourceName - 错误来源名称
 * @returns 可交给 `JSON.parse` 的文本
 */
function stripCommentsAndTrailingCommas(text: string, sourceName: string): string {
  const output: string[] = [];
  let index = 0;
  let isInString = false;
  let stringStart = 0;

  while (index < text.length) {
    const char = text[index];
    if (char === undefined) break;
    if (isInString) {
      const scanned = consumeStringChar(text, index, output, sourceName);
      index = scanned.index;
      isInString = scanned.isInString;
      continue;
    }

    if (char === '"') {
      isInString = true;
      stringStart = index;
      output.push(char);
      index += 1;
      continue;
    }

    if (char === '/' && text[index + 1] === '/') {
      index = skipLineComment(text, index + 2);
      continue;
    }

    if (char === '/' && text[index + 1] === '*') {
      const comment = readBlockComment(text, index, sourceName);
      // 注释在 JSONC 中等价于空白；必须保留一个分隔符，否则 `1/*x*/2` 会合并成 `12`。
      output.push(...comment.newlines, ' ');
      index = comment.index;
      continue;
    }

    if (char === ',' && isTrailingComma(text, index, sourceName)) {
      index += 1;
      continue;
    }

    output.push(char);
    index += 1;
  }

  if (isInString) {
    throw new ConfigSyntaxError(sourceName, '字符串未闭合', stringStart);
  }
  return output.join('');
}

/** 消费字符串内的一个字符（含转义对）。 */
function consumeStringChar(
  text: string,
  index: number,
  output: string[],
  sourceName: string,
): StringScan {
  const char = text[index];
  if (char === undefined) return { index, isInString: true };
  output.push(char);

  if (char === '\\') {
    const escaped = text[index + 1];
    if (escaped === undefined) {
      throw new ConfigSyntaxError(sourceName, '字符串转义符后缺少字符', index);
    }
    output.push(escaped);
    return { index: index + 2, isInString: true };
  }
  return { index: index + 1, isInString: char !== '"' };
}

/** 跳行注释，返回换行符或文本末尾的下标。 */
function skipLineComment(text: string, start: number): number {
  let index = start;
  while (index < text.length && text[index] !== '\n' && text[index] !== '\r') index += 1;
  return index;
}

/** 读取块注释，返回结束下标与其中的换行符。 */
function readBlockComment(text: string, start: number, sourceName: string): BlockCommentScan {
  const end = text.indexOf('*/', start + 2);
  if (end < 0) throw new ConfigSyntaxError(sourceName, '块注释未闭合', start);

  const newlines: string[] = [];
  for (let cursor = start; cursor < end; cursor += 1) {
    if (text[cursor] === '\n') newlines.push('\n');
  }
  return { index: end + 2, newlines };
}

/** 判断逗号后是否直接遇到对象/数组结束符。 */
function isTrailingComma(text: string, commaIndex: number, sourceName: string): boolean {
  const next = findNextSignificantIndex(text, commaIndex + 1, sourceName);
  return next < text.length && (text[next] === '}' || text[next] === ']');
}

/** 跳过空白与注释，返回下一个有效字符下标。 */
function findNextSignificantIndex(text: string, start: number, sourceName: string): number {
  let index = start;
  while (index < text.length) {
    const char = text[index];
    if (char === ' ' || char === '\t' || char === '\n' || char === '\r') {
      index += 1;
      continue;
    }
    if (char === '/' && text[index + 1] === '/') {
      index = skipLineComment(text, index + 2);
      continue;
    }
    if (char === '/' && text[index + 1] === '*') {
      index = readBlockComment(text, index, sourceName).index;
      continue;
    }
    break;
  }
  return index;
}
