/**
 * @file 内存对象配置来源测试。
 * @description 覆盖正常加载与空名称/非对象 value 的异常分支。
 * @module config/source/object-source.test
 */

import { describe, expect, it } from 'vitest';

import { ConfigSchemaDefinitionError } from '../errors.js';
import { createMemoryConfigHost } from './index.js';
import { objectSource } from './object-source.js';

describe('objectSource', () => {
  it('返回名称、种类与原始对象', () => {
    const source = objectSource({ name: 'cli', value: { port: 8080 } });
    const host = createMemoryConfigHost();
    expect(source.name).toBe('cli');
    expect(source.kind).toBe('object');
    expect(source.load({ host, cwd: host.cwd })).toEqual({ port: 8080 });
  });

  it('拒绝空名称、非字符串名称与非普通对象 value', () => {
    expect(() => objectSource({ name: '  ', value: {} })).toThrow(ConfigSchemaDefinitionError);
    expect(() => objectSource(JSON.parse('{"name":1,"value":{}}'))).toThrow(
      ConfigSchemaDefinitionError,
    );
    expect(() => objectSource(JSON.parse('{"name":"x","value":[]}'))).toThrow(
      ConfigSchemaDefinitionError,
    );
  });
});
