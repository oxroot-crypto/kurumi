/**
 * @file 配置字符串插值单元测试。
 * @description 覆盖变量、默认值、转义、缺失变量、嵌套路径与非法占位符。
 * @module config/interpolate.test
 */

import { describe, expect, it } from 'vitest';

import { ConfigError, ConfigInterpolationError } from './errors.js';
import { interpolateConfig } from './interpolate.js';

describe('interpolateConfig', () => {
  it('替换环境变量并支持多个占位符', () => {
    const result = interpolateConfig(
      { url: '${PROTOCOL}://${HOST}:${PORT}/api' },
      { PROTOCOL: 'https', HOST: 'example.com', PORT: '3000' },
    );
    expect(result).toEqual({ url: 'https://example.com:3000/api' });
  });

  it('缺省值生效，空字符串环境变量视为已定义', () => {
    expect(interpolateConfig({ a: '${MISSING:-fallback}' }, {})).toEqual({ a: 'fallback' });
    expect(interpolateConfig({ a: '${EMPTY:-fallback}' }, { EMPTY: '' })).toEqual({ a: '' });
    expect(interpolateConfig({ a: '${MISSING:-}' }, {})).toEqual({ a: '' });
  });

  it('支持 {env:NAME} 与 {env:NAME:-fallback} 形式', () => {
    expect(
      interpolateConfig(
        { url: 'postgres://{env:DB_HOST}:{env:DB_PORT}/app', key: '{env:API_KEY}' },
        { DB_HOST: 'localhost', DB_PORT: '5432', API_KEY: 'secret' },
      ),
    ).toEqual({
      url: 'postgres://localhost:5432/app',
      key: 'secret',
    });
    expect(interpolateConfig({ a: '{env:MISSING:-fallback}' }, {})).toEqual({ a: 'fallback' });
    expect(interpolateConfig({ a: '{env:EMPTY:-fallback}' }, { EMPTY: '' })).toEqual({ a: '' });
  });

  it('混合 ${NAME} 与 {env:NAME} 且环境变量值不递归插值', () => {
    expect(interpolateConfig({ a: '${A}-{env:B}' }, { A: '{env:C}', B: '${C}', C: 'x' })).toEqual({
      a: '{env:C}-${C}',
    });
  });

  it('{env:NAME} 缺失变量时聚合错误并携带路径', () => {
    const error = catchError(() => interpolateConfig({ server: { key: '{env:MISSING}' } }, {}));
    expect(error).toBeInstanceOf(ConfigInterpolationError);
    const issues = (error as ConfigInterpolationError).issues;
    expect(issues).toHaveLength(1);
    expect(issues[0]?.path).toBe('$.server.key');
    expect(issues[0]?.code).toBe('missing_env_var');
  });

  it('{env:} 非法变量名与未闭合占位符按字面量保留', () => {
    expect(interpolateConfig({ a: '{env:1BAD}' }, {})).toEqual({ a: '{env:1BAD}' });
    expect(interpolateConfig({ a: '{env:UNCLOSED' }, {})).toEqual({ a: '{env:UNCLOSED' });
  });

  it('{env:NAME} 在 shouldAllowMissing 时保留原文', () => {
    expect(interpolateConfig({ a: '{env:MISSING}' }, {}, { shouldAllowMissing: true })).toEqual({
      a: '{env:MISSING}',
    });
  });

  it('$${NAME} 转义为字面量且不解析', () => {
    expect(interpolateConfig({ a: '$${KEEP}' }, { KEEP: 'changed' })).toEqual({ a: '${KEEP}' });
    expect(interpolateConfig({ a: 'price: $$5' }, {})).toEqual({ a: 'price: $$5' });
  });

  it('递归处理对象与数组，不处理键', () => {
    const result = interpolateConfig(
      { '${KEY}': ['${A}', { nested: '${B}' }], number: 1, flag: true },
      { A: 'a', B: 'b' },
    );
    expect(result).toEqual({ '${KEY}': ['a', { nested: 'b' }], number: 1, flag: true });
  });

  it('缺失变量时聚合错误并携带路径', () => {
    const error = catchError(() => interpolateConfig({ server: { url: '${HOST}' } }, {}));
    expect(error).toBeInstanceOf(ConfigInterpolationError);
    const issues = (error as ConfigInterpolationError).issues;
    expect(issues).toHaveLength(1);
    expect(issues[0]?.path).toBe('$.server.url');
    expect(issues[0]?.code).toBe('missing_env_var');
  });

  it('shouldAllowMissing 保留原始占位符', () => {
    expect(interpolateConfig({ a: '${HOST}/api' }, {}, { shouldAllowMissing: true })).toEqual({
      a: '${HOST}/api',
    });
  });

  it('非法变量名与未闭合占位符按字面量保留', () => {
    expect(interpolateConfig({ a: '${1BAD}' }, {})).toEqual({ a: '${1BAD}' });
    expect(interpolateConfig({ a: '${UNCLOSED' }, {})).toEqual({ a: '${UNCLOSED' });
  });

  it('环境变量值不会被递归插值', () => {
    expect(interpolateConfig({ a: '${A}' }, { A: '${B}', B: 'x' })).toEqual({ a: '${B}' });
  });

  it('不修改输入层', () => {
    const input = { a: '${A}', nested: ['${A}'] };
    interpolateConfig(input, { A: 'x' });
    expect(input).toEqual({ a: '${A}', nested: ['${A}'] });
  });

  it('拒绝危险键，避免替换输出对象原型', () => {
    const malicious = JSON.parse('{"__proto__":{"polluted":true},"a":"${A}"}');
    const error = catchError(() => interpolateConfig(malicious, { A: 'x' }));
    expect(error).toBeInstanceOf(ConfigInterpolationError);
    expect((error as ConfigInterpolationError).issues[0]?.code).toBe('forbidden_key');
    expect(({} as Record<string, unknown>)['polluted']).toBeUndefined();
  });

  it('超过深度上限时抛出 ConfigError', () => {
    let nested: Record<string, unknown> = { value: '${A}' };
    for (let index = 0; index < 70; index += 1) nested = { next: nested };
    expect(() =>
      interpolateConfig(nested as { readonly [key: string]: unknown }, { A: 'x' }),
    ).toThrow(ConfigError);
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
