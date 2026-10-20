/**
 * @file JSON / JSONC 解析器单元测试。
 * @description 覆盖注释、尾随逗号、BOM、字符串内符号与语法错误。
 * @module config/parser/jsonc.test
 */

import { describe, expect, it } from 'vitest';

import { ConfigSyntaxError } from '../errors.js';
import { parseJson } from './json.js';
import { parseJsonc } from './jsonc.js';

describe('parseJson', () => {
  it('解析严格 JSON 并接受 BOM', () => {
    expect(parseJson('\uFEFF{"a":1}', 'test.json')).toEqual({ a: 1 });
  });

  it('拒绝注释并抛出带来源的语法错误', () => {
    expect(() => parseJson('{"a":1 // comment\n}', 'test.json')).toThrow(ConfigSyntaxError);
  });

  it('保留 cause', () => {
    const error = catchError(() => parseJson('{', 'test.json'));
    expect(error).toBeInstanceOf(ConfigSyntaxError);
    expect((error as ConfigSyntaxError).sourceName).toBe('test.json');
    expect((error as ConfigSyntaxError).cause).toBeInstanceOf(Error);
  });
});

describe('parseJsonc', () => {
  it('移除行注释、块注释与尾随逗号', () => {
    const text = `{
      // 行注释
      "a": 1,
      "b": [1, 2, /* 块注释 */],
      "c": 3,
    }`;
    expect(parseJsonc(text, 'test.jsonc')).toEqual({ a: 1, b: [1, 2], c: 3 });
  });

  it('保留字符串中的注释符号与转义引号', () => {
    const text = '{"url":"https://example.com/a//b","pattern":"/\\\\*.*?\\\\*/","quote":"a\\"b"}';
    expect(parseJsonc(text, 'test.jsonc')).toEqual({
      url: 'https://example.com/a//b',
      pattern: '/\\*.*?\\*/',
      quote: 'a"b',
    });
  });

  it('支持块注释后的尾随逗号', () => {
    expect(parseJsonc('{"a":1, /* trailing */}', 'test.jsonc')).toEqual({ a: 1 });
  });

  it('块注释按空白处理，不会把相邻 token 合并', () => {
    // `[1/*x*/2]` 在 JSONC 中应等价于 `[1 2]`（非法），而不是合并成 `[12]`。
    expect(() => parseJsonc('[1/*x*/2]', 'test.jsonc')).toThrow(ConfigSyntaxError);
    expect(parseJsonc('[1/*x*/,2]', 'test.jsonc')).toEqual([1, 2]);
    expect(parseJsonc('{"a":1/*x*/,"b":2}', 'test.jsonc')).toEqual({ a: 1, b: 2 });
  });

  it('接受 BOM 与空对象', () => {
    expect(parseJsonc('\uFEFF{}', 'test.jsonc')).toEqual({});
  });

  it('块注释未闭合时抛出 ConfigSyntaxError', () => {
    const error = catchError(() => parseJsonc('{"a":1 /*', 'test.jsonc'));
    expect(error).toBeInstanceOf(ConfigSyntaxError);
    expect((error as ConfigSyntaxError).message).toContain('块注释未闭合');
  });

  it('字符串未闭合时抛出 ConfigSyntaxError', () => {
    expect(() => parseJsonc('{"a":"oops}', 'test.jsonc')).toThrow(/字符串未闭合/);
  });

  it('清理后仍非法时抛出带偏移的语法错误', () => {
    const error = catchError(() => parseJsonc('{"a": }', 'test.jsonc'));
    expect(error).toBeInstanceOf(ConfigSyntaxError);
    expect(Number.isInteger((error as ConfigSyntaxError).offset)).toBe(true);
  });
});

function catchError(run: () => unknown): unknown {
  try {
    run();
  } catch (error) {
    return error;
  }
  throw new Error('预期抛出错误，但调用成功');
}
