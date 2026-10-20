/**
 * @file Token 用量契约测试。
 * @description 覆盖构造、归一化、累加、边界与冻结。
 * @module agent/usage.test
 */

import { describe, expect, it } from 'vitest';

import { AgentDefinitionError } from './errors.js';
import {
  EMPTY_TOKEN_USAGE,
  addTokenUsage,
  createTokenUsage,
  normalizeTokenUsage,
} from './index.js';

describe('createTokenUsage', () => {
  it('计算 totalTokens 并冻结', () => {
    const usage = createTokenUsage(12, 7);

    expect(usage).toEqual({ inputTokens: 12, outputTokens: 7, totalTokens: 19 });
    expect(Object.isFrozen(usage)).toBe(true);
  });

  it('保留可选缓存计数', () => {
    const usage = createTokenUsage(10, 5, { cacheReadTokens: 3, cacheWriteTokens: 2 });

    expect(usage).toEqual({
      inputTokens: 10,
      outputTokens: 5,
      totalTokens: 15,
      cacheReadTokens: 3,
      cacheWriteTokens: 2,
    });
  });

  it('接受零值', () => {
    expect(createTokenUsage(0, 0)).toEqual({ inputTokens: 0, outputTokens: 0, totalTokens: 0 });
  });

  it.each([
    ['负数输入', -1, 0],
    ['小数输入', 1.5, 0],
    ['NaN 输入', Number.NaN, 0],
    ['Infinity 输入', Number.POSITIVE_INFINITY, 0],
    ['负数输出', 0, -1],
    ['超出安全整数', Number.MAX_SAFE_INTEGER, 1],
  ])('拒绝%s', (_name, input, output) => {
    expect(() => createTokenUsage(input, output)).toThrow(AgentDefinitionError);
  });

  it('拒绝非法缓存计数', () => {
    expect(() => createTokenUsage(1, 1, { cacheReadTokens: -1 })).toThrow(AgentDefinitionError);
    expect(() => createTokenUsage(1, 1, { cacheWriteTokens: 1.5 })).toThrow(AgentDefinitionError);
  });
});

describe('normalizeTokenUsage', () => {
  it('缺省 totalTokens 时自动计算', () => {
    expect(normalizeTokenUsage({ inputTokens: 2, outputTokens: 3 })).toEqual({
      inputTokens: 2,
      outputTokens: 3,
      totalTokens: 5,
    });
  });

  it('接受一致的 totalTokens', () => {
    expect(normalizeTokenUsage({ inputTokens: 2, outputTokens: 3, totalTokens: 5 })).toEqual({
      inputTokens: 2,
      outputTokens: 3,
      totalTokens: 5,
    });
  });

  it('校验并保留缓存计数', () => {
    expect(
      normalizeTokenUsage({
        inputTokens: 2,
        outputTokens: 3,
        cacheReadTokens: 1,
        cacheWriteTokens: 0,
      }),
    ).toMatchObject({ cacheReadTokens: 1, cacheWriteTokens: 0 });
  });

  it.each([
    ['非对象', null],
    ['缺输入', { outputTokens: 1 }],
    ['缺输出', { inputTokens: 1 }],
    ['负数', { inputTokens: -1, outputTokens: 1 }],
    ['小数', { inputTokens: 1.5, outputTokens: 1 }],
    ['总数不一致', { inputTokens: 1, outputTokens: 1, totalTokens: 3 }],
    ['缓存非法', { inputTokens: 1, outputTokens: 1, cacheReadTokens: -1 }],
  ])('拒绝%s', (_name, value) => {
    expect(() => normalizeTokenUsage(value)).toThrow(AgentDefinitionError);
  });

  it('错误中包含路径', () => {
    expect(() => normalizeTokenUsage({ inputTokens: 1 }, '$.usage')).toThrow(
      '$.usage.outputTokens',
    );
  });
});

describe('addTokenUsage', () => {
  it('累加输入输出与总数', () => {
    const left = createTokenUsage(1, 2);
    const right = createTokenUsage(3, 4);

    expect(addTokenUsage(left, right)).toEqual({
      inputTokens: 4,
      outputTokens: 6,
      totalTokens: 10,
    });
  });

  it('任一侧存在缓存时按 0 补齐', () => {
    const left = createTokenUsage(1, 1, { cacheReadTokens: 2 });
    const right = createTokenUsage(1, 1, { cacheWriteTokens: 3 });
    const usage = addTokenUsage(left, right);

    expect(usage.cacheReadTokens).toBe(2);
    expect(usage.cacheWriteTokens).toBe(3);
  });

  it('两侧均无缓存时不产生缓存字段', () => {
    const usage = addTokenUsage(createTokenUsage(1, 1), createTokenUsage(1, 1));

    expect(usage).not.toHaveProperty('cacheReadTokens');
    expect(usage).not.toHaveProperty('cacheWriteTokens');
  });

  it('累加溢出时抛错', () => {
    const left = createTokenUsage(Number.MAX_SAFE_INTEGER, 0);
    const right = createTokenUsage(1, 0);

    expect(() => addTokenUsage(left, right)).toThrow(AgentDefinitionError);
  });

  it('EMPTY_TOKEN_USAGE 是冻结零值', () => {
    expect(EMPTY_TOKEN_USAGE).toEqual({ inputTokens: 0, outputTokens: 0, totalTokens: 0 });
    expect(Object.isFrozen(EMPTY_TOKEN_USAGE)).toBe(true);
  });
});
