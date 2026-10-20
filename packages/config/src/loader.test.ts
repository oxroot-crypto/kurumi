/**
 * @file 配置加载流水线单元测试。
 * @description 覆盖来源优先级、默认值、环境强转、插值、错误顺序、冻结与并行加载。
 * @module config/loader.test
 */

import { number as baseNumber, object as baseObject } from '@kurumi/schema';
import { describe, expect, it } from 'vitest';

import {
  ConfigError,
  ConfigInterpolationError,
  ConfigSchemaDefinitionError,
  ConfigSourceError,
  ConfigValidationError,
} from './errors.js';
import { loadConfig } from './loader.js';
import { array, boolean, number, object, string, type Schema } from './schema/index.js';
import { ConfigSourceKind, type ConfigSource, type ConfigSourceContext } from './source.js';
import { createMemoryConfigHost, envSource, fileSource, objectSource } from './source/index.js';

const schema = object({
  server: object({
    host: string().default('127.0.0.1'),
    port: number({ isInteger: true, min: 1, max: 65_535 }),
  }),
  debug: boolean().default(false),
});

describe('loadConfig', () => {
  it('按来源顺序覆盖并应用默认值', async () => {
    const host = createMemoryConfigHost({
      cwd: '/app',
      env: { KURUMI_SERVER__HOST: 'env-host' },
      files: {
        'kurumi.json': '{"server":{"port":3000}}',
      },
    });

    const config = await loadConfig({
      schema,
      host,
      sources: [
        objectSource({ name: 'low', value: { server: { host: 'low-host' } } }),
        fileSource({ path: 'kurumi.json' }),
        envSource({ prefix: 'KURUMI_' }),
        objectSource({ name: 'override', value: { server: { host: 'override-host' } } }),
      ],
    });

    expect(config).toEqual({
      server: { host: 'override-host', port: 3000 },
      debug: false,
    });
  });

  it('仅通过环境变量提供 secret 必填字段', async () => {
    const secretSchema = object({ api_key: string({ isNonEmpty: true }).secret() });
    const host = createMemoryConfigHost({ env: { KURUMI_API_KEY: 'sk-env-only' } });

    const config = await loadConfig({
      schema: secretSchema,
      host,
      sources: [envSource({ prefix: 'KURUMI_' })],
    });

    expect(config.api_key).toBe('sk-env-only');
  });

  it('环境变量字符串按 schema 强转为 number/boolean', async () => {
    const host = createMemoryConfigHost({
      env: { KURUMI_SERVER__PORT: '8080', KURUMI_DEBUG: 'true' },
    });
    const config = await loadConfig({
      schema,
      host,
      sources: [envSource({ prefix: 'KURUMI_' })],
    });
    expect(config).toEqual({
      server: { host: '127.0.0.1', port: 8080 },
      debug: true,
    });
  });

  it('没有任何来源时报告必填字段', async () => {
    const error = await catchError(loadConfig({ schema, host: createMemoryConfigHost() }));
    expect(error).toBeInstanceOf(ConfigValidationError);
    const issues = (error as ConfigValidationError).issues;
    expect(issues.map((issue) => issue.path)).toEqual(['$.server.port']);
    expect(issues[0]?.code).toBe('required');
  });

  it('在合并后执行插值，并支持默认值与环境变量', async () => {
    const host = createMemoryConfigHost({
      env: { PORT: '9000', HOST: 'example.com' },
      files: {
        '/app/kurumi.json': '{"url":"${PROTOCOL:-http}://${HOST}:${PORT}","nested":["${HOST}"]}',
      },
    });
    const urlSchema = object({ url: string(), nested: array(string()) });
    const config = await loadConfig({
      schema: urlSchema,
      host,
      sources: [fileSource({ path: '/app/kurumi.json' })],
    });
    expect(config.url).toBe('http://example.com:9000');
    expect(config.nested).toEqual(['example.com']);
  });

  it('缺失插值变量时抛出 ConfigInterpolationError', async () => {
    const host = createMemoryConfigHost({
      files: { '/app/kurumi.json': '{"url":"${MISSING}"}' },
    });
    const error = await catchError(
      loadConfig({
        schema: object({ url: string() }),
        host,
        sources: [fileSource({ path: '/app/kurumi.json' })],
      }),
    );
    expect(error).toBeInstanceOf(ConfigInterpolationError);
  });

  it('interpolation=false 关闭插值', async () => {
    const host = createMemoryConfigHost({
      files: { '/app/kurumi.json': '{"url":"${MISSING}"}' },
    });
    const config = await loadConfig({
      schema: object({ url: string() }),
      host,
      sources: [fileSource({ path: '/app/kurumi.json' })],
      interpolation: false,
    });
    expect(config.url).toBe('${MISSING}');
  });

  it('shouldAllowMissing 保留占位符', async () => {
    const host = createMemoryConfigHost({
      files: { '/app/kurumi.json': '{"url":"${MISSING}"}' },
    });
    const config = await loadConfig({
      schema: object({ url: string() }),
      host,
      sources: [fileSource({ path: '/app/kurumi.json' })],
      interpolation: { shouldAllowMissing: true },
    });
    expect(config.url).toBe('${MISSING}');
  });

  it('返回结果被递归冻结', async () => {
    const config = await loadConfig({
      schema,
      host: createMemoryConfigHost(),
      sources: [objectSource({ name: 'base', value: { server: { port: 1 } } })],
    });
    expect(Object.isFrozen(config)).toBe(true);
    expect(Object.isFrozen(config.server)).toBe(true);
  });

  it('拒绝重复来源名称与空名称', async () => {
    const host = createMemoryConfigHost();
    await expect(
      loadConfig({
        schema,
        host,
        sources: [
          objectSource({ name: 'same', value: {} }),
          objectSource({ name: 'same', value: {} }),
        ],
      }),
    ).rejects.toBeInstanceOf(ConfigSchemaDefinitionError);

    const emptyName: ConfigSource = {
      name: '',
      kind: ConfigSourceKind.Object,
      load: () => ({}),
    };
    await expect(loadConfig({ schema, host, sources: [emptyName] })).rejects.toBeInstanceOf(
      ConfigSchemaDefinitionError,
    );
  });

  it('来源异常按声明顺序返回，并保留 cause', async () => {
    const host = createMemoryConfigHost();
    const failing = (name: string, message: string): ConfigSource => ({
      name,
      kind: ConfigSourceKind.Object,
      load: () => {
        throw new Error(message);
      },
    });
    const error = await catchError(
      loadConfig({ schema, host, sources: [failing('first', 'one'), failing('second', 'two')] }),
    );
    expect(error).toBeInstanceOf(ConfigSourceError);
    expect((error as ConfigSourceError).sourceName).toBe('first');
    expect((error as ConfigSourceError).cause).toBeInstanceOf(Error);
  });

  it('来源返回非对象时抛出 invalid_layer', async () => {
    const host = createMemoryConfigHost();
    const invalid: ConfigSource = {
      name: 'array',
      kind: ConfigSourceKind.Object,
      load: () => [] as never,
    };
    const error = await catchError(loadConfig({ schema, host, sources: [invalid] }));
    expect(error).toBeInstanceOf(ConfigSourceError);
    expect((error as ConfigSourceError).code).toBe('invalid_layer');
  });

  it('undefined 来源被跳过', async () => {
    const skipped: ConfigSource = {
      name: 'skipped',
      kind: ConfigSourceKind.Object,
      load: () => undefined,
    };
    const config = await loadConfig({
      schema,
      host: createMemoryConfigHost(),
      sources: [skipped, objectSource({ name: 'base', value: { server: { port: 1 } } })],
    });
    expect(config.server.port).toBe(1);
  });

  it('独立来源并行加载', async () => {
    const contextGate = createGate();
    let started = 0;
    const makeSource = (name: string): ConfigSource => ({
      name,
      kind: ConfigSourceKind.Object,
      async load(_context: ConfigSourceContext) {
        started += 1;
        if (started === 2) contextGate.open();
        await contextGate.promise;
        return {};
      },
    });

    const promise = loadConfig({
      schema: object({}),
      host: createMemoryConfigHost(),
      sources: [makeSource('a'), makeSource('b')],
    });
    await contextGate.promise;
    await expect(promise).resolves.toEqual({});
    expect(started).toBe(2);
  });

  it('通用 schema 校验失败统一转换为 ConfigValidationError', async () => {
    const baseSchema = baseObject({ port: baseNumber() });
    const error = await catchError(
      loadConfig({ schema: baseSchema, host: createMemoryConfigHost() }),
    );
    expect(error).toBeInstanceOf(ConfigValidationError);
  });

  it('拒绝非 Schema 实例', async () => {
    await expect(
      loadConfig({
        schema: {} as Schema<object>,
        host: createMemoryConfigHost(),
      }),
    ).rejects.toBeInstanceOf(ConfigSchemaDefinitionError);
  });

  it('拒绝非法 host、sources 与 interpolation', async () => {
    await expect(
      loadConfig({ schema: object({}), host: JSON.parse('null') }),
    ).rejects.toBeInstanceOf(ConfigSchemaDefinitionError);
    await expect(
      loadConfig({ schema: object({}), host: createMemoryConfigHost(), sources: JSON.parse('{}') }),
    ).rejects.toBeInstanceOf(ConfigSchemaDefinitionError);
    await expect(
      loadConfig({
        schema: object({}),
        host: createMemoryConfigHost(),
        interpolation: JSON.parse('true'),
      }),
    ).rejects.toBeInstanceOf(ConfigSchemaDefinitionError);
    await expect(
      loadConfig({
        schema: object({}),
        host: createMemoryConfigHost(),
        signal: JSON.parse('{}'),
      }),
    ).rejects.toBeInstanceOf(ConfigSchemaDefinitionError);
  });

  it('把 AbortSignal 传给每个来源上下文', async () => {
    const controller = new AbortController();
    let received: AbortSignal | undefined;
    const source: ConfigSource = {
      name: 'signal-aware',
      kind: ConfigSourceKind.Object,
      load: (context) => {
        received = context.signal;
        return {};
      },
    };
    await loadConfig({
      schema: object({}),
      host: createMemoryConfigHost(),
      sources: [source],
      signal: controller.signal,
    });
    expect(received).toBe(controller.signal);
  });

  it('已取消的 AbortSignal 在加载前被拒绝', async () => {
    const controller = new AbortController();
    controller.abort();
    const error = await catchError(
      loadConfig({
        schema: object({}),
        host: createMemoryConfigHost(),
        signal: controller.signal,
      }),
    );
    expect((error as Error).name).toBe('AbortError');
  });

  it('合并阶段的原型污染错误会向上抛出', async () => {
    const malicious = JSON.parse('{"__proto__":{"polluted":true}}') as Record<string, unknown>;
    const error = await catchError(
      loadConfig({
        schema: object({}),
        host: createMemoryConfigHost(),
        sources: [objectSource({ name: 'malicious', value: malicious })],
      }),
    );
    expect(error).toBeInstanceOf(ConfigError);
    expect(({} as Record<string, unknown>)['polluted']).toBeUndefined();
  });
});

async function catchError(promise: Promise<unknown>): Promise<unknown> {
  try {
    return await promise;
  } catch (error) {
    return error;
  }
}

function createGate(): { readonly promise: Promise<void>; readonly open: () => void } {
  const { promise, resolve } = Promise.withResolvers<void>();
  return {
    promise,
    open: () => {
      resolve();
    },
  };
}
