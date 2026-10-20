/**
 * @file Agent 数据契约属性测试。
 * @description 用随机文本与 JSON 验证消息归一化和工具输入克隆的不变量。
 * @module agent/properties.test
 */

import fc from 'fast-check';
import { describe, expect, it } from 'vitest';

import { isDeepEqual, isForbiddenKey, type JsonValue } from '@kurumi/core';

import { createAssistantMessage, extractMessageText, getToolCalls } from './index.js';

function sanitize(value: unknown): unknown {
  if (Array.isArray(value)) return value.map((item) => sanitize(item));
  if (typeof value === 'object' && value !== null) {
    const result: Record<string, unknown> = {};
    for (const [key, item] of Object.entries(value)) {
      if (!isForbiddenKey(key)) result[key] = sanitize(item);
    }
    return result;
  }
  return value;
}

const safeJson = fc.jsonValue({ maxDepth: 4 }).map((value) => sanitize(value));

describe('消息属性', () => {
  it('任意文本创建后保持内容且冻结', () => {
    fc.assert(
      fc.property(fc.string(), (text) => {
        const message = createAssistantMessage(text);
        expect(extractMessageText(message)).toBe(text);
        expect(Object.isFrozen(message)).toBe(true);
        expect(Object.isFrozen(message.content[0])).toBe(true);
      }),
      { numRuns: 100 },
    );
  });

  it('任意 JSON 工具输入被深拷贝、冻结且结构不变', () => {
    fc.assert(
      fc.property(safeJson, (input) => {
        const message = createAssistantMessage([
          { type: 'tool_call', callId: 'c1', name: 'echo', input: input as JsonValue },
        ]);
        const call = getToolCalls(message)[0];

        expect(call).toBeDefined();
        expect(isDeepEqual(call?.input, input)).toBe(true);
        expect(Object.isFrozen(call?.input)).toBe(true);
      }),
      { numRuns: 100 },
    );
  });
});
