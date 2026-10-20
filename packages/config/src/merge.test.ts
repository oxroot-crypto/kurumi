/**
 * @file 配置层合并单元测试。
 * @description 覆盖深合并、替换语义、不可变性、原型污染与深度边界。
 * @module config/merge.test
 */

import { describe, expect, it } from 'vitest';

import { ConfigError } from './errors.js';
import { mergeConfigLayers } from './merge.js';
import type { ConfigLayer } from './source.js';

describe('mergeConfigLayers', () => {
  it('递归合并对象，后者覆盖前者', () => {
    const low = { server: { host: '0.0.0.0', port: 3000 }, debug: true };
    const high = { server: { port: 8080 } };
    expect(mergeConfigLayers([low, high])).toEqual({
      server: { host: '0.0.0.0', port: 8080 },
      debug: true,
    });
  });

  it('数组、标量与 null 整体替换', () => {
    const merged = mergeConfigLayers([
      { list: [1, 2], value: 'a', nullable: { a: 1 } },
      { list: [3], value: 1, nullable: null },
    ]);
    expect(merged).toEqual({ list: [3], value: 1, nullable: null });
  });

  it('undefined 不覆盖已有值', () => {
    const merged = mergeConfigLayers([
      { a: 1, nested: { b: 2 } },
      { a: undefined, nested: { b: undefined } },
    ]);
    expect(merged).toEqual({ a: 1, nested: { b: 2 } });
  });

  it('不修改任何输入层', () => {
    const low = { server: { host: 'a' } };
    const high = { server: { port: 1 } };
    const beforeLow = structuredClone(low);
    const beforeHigh = structuredClone(high);
    mergeConfigLayers([low, high]);
    expect(low).toEqual(beforeLow);
    expect(high).toEqual(beforeHigh);
  });

  it('拒绝非普通对象配置层', () => {
    expect(() => mergeConfigLayers([[] as never])).toThrow(ConfigError);
    expect(() => mergeConfigLayers([null as never])).toThrow(ConfigError);
  });

  it('拒绝原型污染键', () => {
    const malicious = JSON.parse('{"__proto__":{"polluted":true}}') as ConfigLayer;
    expect(() => mergeConfigLayers([malicious])).toThrow(/危险键/);
    expect(({} as Record<string, unknown>)['polluted']).toBeUndefined();
  });

  it('拒绝函数、Date 与稀疏数组', () => {
    expect(() => mergeConfigLayers([{ fn: () => 1 } as ConfigLayer])).toThrow(
      /不支持的 JSON 值类型/,
    );
    expect(() => mergeConfigLayers([{ date: new Date() } as ConfigLayer])).toThrow(
      /不支持的 JSON 值类型/,
    );
    const sparse: number[] = [];
    sparse.length = 2;
    sparse[1] = 1;
    expect(() => mergeConfigLayers([{ sparse } as ConfigLayer])).toThrow(/空槽/);
  });

  it('超过深度上限时抛出 max_depth_exceeded', () => {
    let nested: Record<string, unknown> = { value: 1 };
    for (let index = 0; index < 70; index += 1) nested = { next: nested };
    expect(() => mergeConfigLayers([nested as ConfigLayer])).toThrow(/嵌套深度超过/);
  });

  it('空层数组返回空对象', () => {
    expect(mergeConfigLayers([])).toEqual({});
  });
});
