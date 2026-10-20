/**
 * @file core/value 单元测试。
 * @description 覆盖 JSON 值判定、克隆、冻结与深比较的边界与异常。
 * @module core/value.test
 */

import { describe, expect, it } from 'vitest';

import {
  MAX_JSON_DEPTH,
  cloneJsonValue,
  deepFreeze,
  isDeepEqual,
  isForbiddenKey,
  isPlainObject,
} from './value.js';

describe('isPlainObject', () => {
  it('接受对象字面量与 null 原型对象', () => {
    expect(isPlainObject({})).toBe(true);
    expect(isPlainObject(Object.create(null))).toBe(true);
  });

  it('拒绝数组、null、Date 与原始值', () => {
    expect(isPlainObject([])).toBe(false);
    expect(isPlainObject(null)).toBe(false);
    expect(isPlainObject(new Date())).toBe(false);
    expect(isPlainObject('object')).toBe(false);
  });
});

describe('isForbiddenKey', () => {
  it('识别原型污染键', () => {
    expect(isForbiddenKey('__proto__')).toBe(true);
    expect(isForbiddenKey('prototype')).toBe(true);
    expect(isForbiddenKey('constructor')).toBe(true);
    expect(isForbiddenKey('safe')).toBe(false);
  });
});

describe('cloneJsonValue', () => {
  it('深拷贝嵌套对象与数组且不共享引用', () => {
    const input = { server: { ports: [1, 2] } };
    const cloned = cloneJsonValue(input) as typeof input;
    expect(cloned).toEqual(input);
    expect(cloned).not.toBe(input);
    expect(cloned.server).not.toBe(input.server);
    expect(cloned.server.ports).not.toBe(input.server.ports);
  });

  it('拒绝函数、Date 与稀疏数组', () => {
    expect(() => cloneJsonValue({ fn: () => 1 })).toThrow(/不支持的 JSON 值类型/);
    expect(() => cloneJsonValue({ date: new Date() })).toThrow(/不支持的 JSON 值类型/);
    const sparse: number[] = [];
    sparse.length = 2;
    sparse[1] = 1;
    expect(() => cloneJsonValue(sparse)).toThrow(/空槽/);
  });

  it('拒绝危险键并保留路径', () => {
    const malicious = JSON.parse('{"__proto__":{"polluted":true}}') as Record<string, unknown>;
    expect(() => cloneJsonValue(malicious)).toThrow(/危险键 "__proto__"/);
  });

  it('超过深度上限时抛出 max_depth_exceeded', () => {
    let nested: unknown = 1;
    for (let index = 0; index <= MAX_JSON_DEPTH; index += 1) nested = { next: nested };
    expect(() => cloneJsonValue(nested)).toThrow(/嵌套深度超过/);
  });

  it('接受等于深度上限的嵌套', () => {
    let nested: unknown = 1;
    for (let index = 0; index < MAX_JSON_DEPTH; index += 1) nested = { next: nested };
    expect(cloneJsonValue(nested)).toEqual(nested);
  });

  it('拒绝非法内部深度参数，避免绕过深度上限', () => {
    expect(() => cloneJsonValue({ a: 1 }, '$', -1)).toThrow(/递归深度/);
    expect(() => cloneJsonValue({ a: 1 }, '$', 1.5)).toThrow(/递归深度/);
  });

  it('拒绝符号键，避免静默丢弃非 JSON 属性', () => {
    expect(() => cloneJsonValue({ [Symbol('secret')]: 1 })).toThrow(/符号键/);
  });

  it('拒绝数组上的非下标属性', () => {
    const value = [1, 2];
    Reflect.set(value, 'extra', true);
    expect(() => cloneJsonValue(value)).toThrow(/非下标属性/);
  });

  it('拒绝非有限数值，避免 JSON 序列化静默变成 null', () => {
    expect(() => cloneJsonValue(Number.NaN)).toThrow(/有限数/);
    expect(() => cloneJsonValue(Number.POSITIVE_INFINITY)).toThrow(/有限数/);
    expect(() => cloneJsonValue({ nested: Number.NEGATIVE_INFINITY })).toThrow(/有限数/);
  });
});

describe('deepFreeze', () => {
  it('递归冻结对象与数组并返回原引用', () => {
    const value = { list: [{ ok: true }] };
    const frozen = deepFreeze(value);
    expect(frozen).toBe(value);
    expect(Object.isFrozen(value)).toBe(true);
    expect(Object.isFrozen(value.list)).toBe(true);
    expect(Object.isFrozen(value.list[0])).toBe(true);
  });

  it('对原始值无副作用', () => {
    expect(deepFreeze(1)).toBe(1);
    expect(deepFreeze('x')).toBe('x');
  });

  it('深层结构使用迭代遍历，不触发调用栈溢出', () => {
    const root: Record<string, unknown> = {};
    let cursor = root;
    for (let index = 0; index < 5000; index += 1) {
      const next: Record<string, unknown> = {};
      cursor['next'] = next;
      cursor = next;
    }
    deepFreeze(root);
    expect(Object.isFrozen(root)).toBe(true);
    expect(Object.isFrozen(cursor)).toBe(true);
  });
});

describe('isDeepEqual', () => {
  it('忽略对象键顺序并比较嵌套数组', () => {
    expect(isDeepEqual({ a: 1, b: [1, 2] }, { b: [1, 2], a: 1 })).toBe(true);
    expect(isDeepEqual({ a: [1, 2] }, { a: [2, 1] })).toBe(false);
  });

  it('区分类型与键集合', () => {
    expect(isDeepEqual({ a: 1 }, { a: '1' })).toBe(false);
    expect(isDeepEqual({ a: 1 }, { a: 1, b: 2 })).toBe(false);
    expect(isDeepEqual(null, {})).toBe(false);
  });

  it('遵循 Object.is 语义', () => {
    expect(isDeepEqual(Number.NaN, Number.NaN)).toBe(true);
    expect(isDeepEqual(-0, 0)).toBe(false);
  });

  it('拒绝把非普通对象视为相等', () => {
    expect(isDeepEqual(new Date(0), new Date(0))).toBe(false);
  });

  it('处理循环引用而不会栈溢出', () => {
    const left: Record<string, unknown> = {};
    const right: Record<string, unknown> = {};
    left['self'] = left;
    right['self'] = right;
    expect(isDeepEqual(left, right)).toBe(true);

    const mismatch: Record<string, unknown> = { self: {} };
    expect(isDeepEqual(left, mismatch)).toBe(false);
  });

  it('深层结构使用迭代比较，不触发调用栈溢出', () => {
    const left: Record<string, unknown> = {};
    const right: Record<string, unknown> = {};
    let leftCursor = left;
    let rightCursor = right;
    for (let index = 0; index < 20_000; index += 1) {
      const nextLeft: Record<string, unknown> = {};
      const nextRight: Record<string, unknown> = {};
      leftCursor['next'] = nextLeft;
      rightCursor['next'] = nextRight;
      leftCursor = nextLeft;
      rightCursor = nextRight;
    }
    expect(isDeepEqual(left, right)).toBe(true);
  });
});
