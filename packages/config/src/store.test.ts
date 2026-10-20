/**
 * @file 配置存储单元测试。
 * @description 覆盖初始加载、变更广播、去重、失败恢复、并发 reload、监听器错误与销毁。
 * @module config/store.test
 */

import { describe, expect, it } from 'vitest';

import { ConfigDisposedError, ConfigError, ConfigValidationError } from './errors.js';
import { number, object } from './schema/index.js';
import { ConfigSourceKind, type ConfigSource, type ConfigSourceContext } from './source.js';
import { createMemoryConfigHost } from './source/index.js';
import { ConfigStoreStatus, createConfigStore } from './store.js';

interface DynamicSource {
  readonly source: ConfigSource;
  set(value: unknown): void;
  readonly loads: () => number;
}

function createDynamicSource(initial: unknown): DynamicSource {
  let value = initial;
  let loads = 0;
  return {
    source: {
      name: 'dynamic',
      kind: ConfigSourceKind.Object,
      load: () => {
        loads += 1;
        return { value };
      },
    },
    set: (next) => {
      value = next;
    },
    loads: () => loads,
  };
}

const schema = object({ value: number() });
const host = createMemoryConfigHost();

describe('createConfigStore', () => {
  it('完成初始加载并处于 Ready', async () => {
    const dynamic = createDynamicSource(1);
    const store = await createConfigStore({ schema, host, sources: [dynamic.source] });
    expect(store.status).toBe(ConfigStoreStatus.Ready);
    expect(store.get()).toEqual({ value: 1 });
    expect(store.lastError).toBeUndefined();
    expect(dynamic.loads()).toBe(1);
  });

  it('reload 后广播新旧配置', async () => {
    const dynamic = createDynamicSource(1);
    const store = await createConfigStore({ schema, host, sources: [dynamic.source] });
    const changes: Array<{ config: unknown; previous: unknown }> = [];
    store.subscribe((change) => {
      changes.push(change);
    });

    dynamic.set(2);
    await expect(store.reload()).resolves.toEqual({ value: 2 });
    expect(changes).toEqual([{ config: { value: 2 }, previous: { value: 1 } }]);
    expect(store.get()).toEqual({ value: 2 });
  });

  it('结构相等时不广播', async () => {
    const dynamic = createDynamicSource({ nested: { value: 1 } });
    const nestedSchema = object({ value: object({ nested: object({ value: number() }) }) });
    const store = await createConfigStore({
      schema: nestedSchema,
      host,
      sources: [dynamic.source],
    });
    let notifications = 0;
    store.subscribe(() => {
      notifications += 1;
    });
    await store.reload();
    expect(notifications).toBe(0);
  });

  it('支持自定义相等判断', async () => {
    const dynamic = createDynamicSource(1);
    const store = await createConfigStore({
      schema,
      host,
      sources: [dynamic.source],
      equals: () => true,
    });
    let notifications = 0;
    store.subscribe(() => {
      notifications += 1;
    });
    dynamic.set(2);
    await store.reload();
    expect(notifications).toBe(0);
  });

  it('取消订阅幂等', async () => {
    const dynamic = createDynamicSource(1);
    const store = await createConfigStore({ schema, host, sources: [dynamic.source] });
    let notifications = 0;
    const unsubscribe = store.subscribe(() => {
      notifications += 1;
    });
    unsubscribe();
    unsubscribe();
    dynamic.set(2);
    await store.reload();
    expect(notifications).toBe(0);
  });

  it('拒绝非函数 listener', async () => {
    const dynamic = createDynamicSource(1);
    const store = await createConfigStore({ schema, host, sources: [dynamic.source] });
    expect(() => store.subscribe(JSON.parse('1'))).toThrow(ConfigError);
  });

  it('reload 失败时保留上一份配置并记录 lastError，随后可恢复', async () => {
    const dynamic = createDynamicSource(1);
    const store = await createConfigStore({ schema, host, sources: [dynamic.source] });

    dynamic.set('bad');
    await expect(store.reload()).rejects.toBeInstanceOf(ConfigValidationError);
    expect(store.status).toBe(ConfigStoreStatus.Error);
    expect(store.get()).toEqual({ value: 1 });
    expect(store.lastError).toBeInstanceOf(ConfigValidationError);

    dynamic.set(3);
    await expect(store.reload()).resolves.toEqual({ value: 3 });
    expect(store.status).toBe(ConfigStoreStatus.Ready);
    expect(store.lastError).toBeUndefined();
  });

  it('初始加载失败时不返回 store', async () => {
    const dynamic = createDynamicSource('bad');
    await expect(
      createConfigStore({ schema, host, sources: [dynamic.source] }),
    ).rejects.toBeInstanceOf(ConfigValidationError);
  });

  it('并发 reload 复用同一个 Promise 且只加载一次', async () => {
    const gate = createGate();
    let loads = 0;
    let shouldWait = false;
    const source: ConfigSource = {
      name: 'gated',
      kind: ConfigSourceKind.Object,
      async load(_context: ConfigSourceContext) {
        loads += 1;
        if (shouldWait) await gate.promise;
        return { value: 1 };
      },
    };
    const store = await createConfigStore({ schema, host, sources: [source] });
    expect(loads).toBe(1);

    shouldWait = true;
    const first = store.reload();
    const second = store.reload();
    expect(second).toBe(first);
    gate.open();
    await Promise.all([first, second]);
    expect(loads).toBe(2);
    expect(store.status).toBe(ConfigStoreStatus.Ready);
  });

  it('销毁后状态为 Disposed，subscribe/reload 抛错', async () => {
    const dynamic = createDynamicSource(1);
    const store = await createConfigStore({ schema, host, sources: [dynamic.source] });
    let notifications = 0;
    store.subscribe(() => {
      notifications += 1;
    });

    store.dispose();
    store.dispose();
    expect(store.status).toBe(ConfigStoreStatus.Disposed);
    expect(store.get()).toEqual({ value: 1 });
    expect(() => store.subscribe(() => undefined)).toThrow(ConfigDisposedError);
    await expect(store.reload()).rejects.toBeInstanceOf(ConfigDisposedError);

    dynamic.set(2);
    expect(notifications).toBe(0);
  });

  it('飞行中 dispose 不广播，reload 仍完成并更新当前配置', async () => {
    const gate = createGate();
    let currentValue = 1;
    let shouldWait = false;
    const source: ConfigSource = {
      name: 'gated-dispose',
      kind: ConfigSourceKind.Object,
      async load(_context: ConfigSourceContext) {
        if (shouldWait) await gate.promise;
        return { value: currentValue };
      },
    };
    const store = await createConfigStore({ schema, host, sources: [source] });
    let notifications = 0;
    store.subscribe(() => {
      notifications += 1;
    });

    shouldWait = true;
    currentValue = 2;
    const reloading = store.reload();
    store.dispose();
    gate.open();
    await expect(reloading).resolves.toEqual({ value: 2 });
    expect(notifications).toBe(0);
    expect(store.status).toBe(ConfigStoreStatus.Disposed);
    expect(store.get()).toEqual({ value: 2 });
  });

  it('监听器异常默认聚合为 AggregateError，配置仍已更新', async () => {
    const dynamic = createDynamicSource(1);
    const store = await createConfigStore({ schema, host, sources: [dynamic.source] });
    store.subscribe(() => {
      throw new Error('listener boom');
    });

    dynamic.set(2);
    const error = await catchAsync(() => store.reload());
    expect(error).toBeInstanceOf(AggregateError);
    expect(store.get()).toEqual({ value: 2 });
    expect(store.status).toBe(ConfigStoreStatus.Ready);
    expect(store.lastError).toBeUndefined();
  });

  it('onListenerError 接管异常后 reload 正常完成', async () => {
    const dynamic = createDynamicSource(1);
    const received: unknown[] = [];
    const store = await createConfigStore({
      schema,
      host,
      sources: [dynamic.source],
      onListenerError: async (error) => {
        await Promise.resolve();
        received.push(error);
      },
    });
    store.subscribe(() => {
      throw new Error('listener boom');
    });

    dynamic.set(2);
    await expect(store.reload()).resolves.toEqual({ value: 2 });
    expect(received).toHaveLength(1);
    expect(received[0]).toMatchObject({ message: 'listener boom' });
  });

  it('onListenerError 自身 rejection 被聚合为 AggregateError', async () => {
    const dynamic = createDynamicSource(1);
    const store = await createConfigStore({
      schema,
      host,
      sources: [dynamic.source],
      onListenerError: async () => {
        await Promise.resolve();
        throw new Error('handler boom');
      },
    });
    store.subscribe(() => {
      throw new Error('listener boom');
    });

    dynamic.set(2);
    await expect(store.reload()).rejects.toBeInstanceOf(AggregateError);
    expect(store.get()).toEqual({ value: 2 });
  });

  it('异步 listener 会被等待，rejection 聚合为 AggregateError', async () => {
    const dynamic = createDynamicSource(1);
    const store = await createConfigStore({ schema, host, sources: [dynamic.source] });
    store.subscribe(async () => {
      await Promise.resolve();
      throw new Error('async listener boom');
    });

    dynamic.set(2);
    await expect(store.reload()).rejects.toBeInstanceOf(AggregateError);
    expect(store.get()).toEqual({ value: 2 });
  });

  it('reload 接受 AbortSignal 并保留上一份配置', async () => {
    const dynamic = createDynamicSource(1);
    const store = await createConfigStore({ schema, host, sources: [dynamic.source] });
    const controller = new AbortController();
    controller.abort();

    const error = await catchAsync(() => store.reload(controller.signal));
    expect(error).toBeInstanceOf(ConfigError);
    expect((error as ConfigError).code).toBe('load_error');
    expect(store.get()).toEqual({ value: 1 });
  });

  it('监听器内部 dispose 后不再通知后续监听器', async () => {
    const dynamic = createDynamicSource(1);
    const store = await createConfigStore({ schema, host, sources: [dynamic.source] });
    let laterCalled = false;
    store.subscribe(() => {
      store.dispose();
    });
    store.subscribe(() => {
      laterCalled = true;
    });

    dynamic.set(2);
    await store.reload();
    expect(laterCalled).toBe(false);
    expect(store.status).toBe('disposed');
  });

  it('equals 抛错时包装为 ConfigError 并保留旧配置', async () => {
    const dynamic = createDynamicSource(1);
    const store = await createConfigStore({
      schema,
      host,
      sources: [dynamic.source],
      equals: () => {
        throw new Error('equals boom');
      },
    });

    dynamic.set(2);
    const error = await catchAsync(() => store.reload());
    expect(error).toBeInstanceOf(ConfigError);
    expect((error as ConfigError).code).toBe('compare_error');
    expect(store.get()).toEqual({ value: 1 });
    expect(store.status).toBe('error');
    expect(store.lastError).toBe(error);
  });
});

function createGate(): { readonly promise: Promise<void>; readonly open: () => void } {
  const { promise, resolve } = Promise.withResolvers<void>();
  return {
    promise,
    open: () => {
      resolve();
    },
  };
}

async function catchAsync(run: () => Promise<unknown>): Promise<unknown> {
  try {
    return await run();
  } catch (error) {
    return error;
  }
}
