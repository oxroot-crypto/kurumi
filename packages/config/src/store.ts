/**
 * @file 配置存储：持有当前配置、串行化 reload 并向订阅者广播变更。
 * @description 初始加载失败不会返回 store；reload 失败时保留上一份可用配置并记录 lastError。
 * @module config/store
 */

import { isDeepEqual } from '@kurumi/core';

import { ConfigDisposedError, ConfigError, ConfigSchemaDefinitionError } from './errors.js';
import { loadConfig, assertConfigHost, assertSources, type LoadConfigOptions } from './loader.js';
import { Schema } from './schema/index.js';

/** 配置存储状态。 */
export const ConfigStoreStatus = Object.freeze({
  /** 尚未开始加载。 */
  Idle: 'idle',
  /** 正在加载。 */
  Loading: 'loading',
  /** 已持有可用配置。 */
  Ready: 'ready',
  /** 最近一次 reload 失败；上一份配置仍可读取。 */
  Error: 'error',
  /** 已销毁。 */
  Disposed: 'disposed',
} as const);

/** 配置存储状态联合类型。 */
export type ConfigStoreStatus = (typeof ConfigStoreStatus)[keyof typeof ConfigStoreStatus];

/** 一次配置变更。 */
export interface ConfigChange<TConfig extends object> {
  /** 新配置。 */
  readonly config: TConfig;
  /** 变更前的配置。 */
  readonly previous: TConfig;
}

/** 变更监听器；支持异步，`reload()` 会等待全部监听器完成。 */
export type ConfigChangeListener<TConfig extends object> = (
  change: ConfigChange<TConfig>,
) => void | Promise<void>;

/** 配置存储选项。 */
export interface ConfigStoreOptions<TConfig extends object> extends LoadConfigOptions<TConfig> {
  /**
   * 新旧配置相等判断，缺省深比较。
   * 返回 true 时不触发订阅者。
   */
  readonly equals?: (left: TConfig, right: TConfig) => boolean;
  /**
   * 监听器异常处理；未提供时 reload 会抛出包含全部异常的 AggregateError。
   * 支持异步处理器；其 rejection 同样会被聚合。
   */
  readonly onListenerError?: (error: unknown) => void | Promise<void>;
}

/** 配置存储接口。 */
export interface ConfigStore<TConfig extends object> {
  /**
   * 当前配置；初始加载成功后始终可读（即使 reload 失败）。
   *
   * @returns 最近一次成功加载的配置
   * @throws {ConfigError} 内部状态未初始化（正常情况下不可达）
   */
  get(): TConfig;
  /** 当前状态。 */
  readonly status: ConfigStoreStatus;
  /** 最近一次 reload 失败原因；成功后清空。 */
  readonly lastError: ConfigError | undefined;
  /**
   * 重新加载；并发调用会复用同一个 Promise；signal 只对首次发起的加载生效。
   *
   * @param signal - 可选取消信号
   * @returns 新配置
   * @throws {ConfigError} 加载、校验或比较失败
   */
  reload(signal?: AbortSignal): Promise<TConfig>;
  /**
   * 订阅配置变更。
   *
   * @param listener - 变更监听器
   * @returns 幂等取消函数
   * @throws {ConfigDisposedError} store 已销毁
   * @throws {ConfigError} listener 不是函数
   */
  subscribe(listener: ConfigChangeListener<TConfig>): () => void;
  /**
   * 销毁 store：清空监听器，之后 reload/subscribe 抛错。
   */
  dispose(): void;
}

/**
 * 创建并完成首次加载的配置存储。
 *
 * @param options - 与 `loadConfig` 相同，另可指定相等判断与监听器错误处理
 * @returns 已处于 Ready 状态的 store
 * @throws {ConfigValidationError | ConfigSourceError | ConfigError} 首次加载失败
 *
 * @example
 * ```ts
 * import { createConfigStore } from '@kurumi/config';
 *
 * const store = await createConfigStore({ schema, host, sources });
 * const unsubscribe = store.subscribe(({ config }) => {
 *   // 应用层在此同步新配置；config 包自身不输出日志。
 *   void config;
 * });
 * await store.reload();
 * unsubscribe();
 * store.dispose();
 * ```
 */
export async function createConfigStore<TConfig extends object>(
  options: ConfigStoreOptions<TConfig>,
): Promise<ConfigStore<TConfig>> {
  if (typeof options !== 'object' || options === null) {
    throw new ConfigSchemaDefinitionError('createConfigStore options 必须是对象');
  }
  if (!(options.schema instanceof Schema)) {
    throw new ConfigSchemaDefinitionError(
      'createConfigStore 需要传入 @kurumi/config 创建的 Schema 实例',
    );
  }
  assertConfigHost(options.host);
  assertSources(options.sources);
  if (options.equals !== undefined && typeof options.equals !== 'function') {
    throw new ConfigSchemaDefinitionError('createConfigStore equals 必须是函数');
  }
  if (options.onListenerError !== undefined && typeof options.onListenerError !== 'function') {
    throw new ConfigSchemaDefinitionError('createConfigStore onListenerError 必须是函数');
  }
  // 快照来源列表，避免 store 创建后调用方修改数组导致 reload 行为漂移。
  const sources = options.sources === undefined ? undefined : Object.freeze([...options.sources]);
  const loadOptions: LoadConfigOptions<TConfig> = {
    schema: options.schema,
    host: options.host,
    ...(sources === undefined ? {} : { sources }),
    ...(options.interpolation === undefined
      ? {}
      : {
          interpolation:
            options.interpolation === false ? false : Object.freeze({ ...options.interpolation }),
        }),
  };
  const store = new DefaultConfigStore<TConfig>({
    load: async (signal) =>
      loadConfig({
        ...loadOptions,
        ...(signal === undefined ? {} : { signal }),
      }),
    equals: options.equals ?? isDeepEqual,
    ...(options.onListenerError === undefined ? {} : { onListenerError: options.onListenerError }),
  });
  await store.initialize(options.signal);
  return store;
}

interface StoreInternals<TConfig extends object> {
  readonly load: (signal?: AbortSignal) => Promise<TConfig>;
  readonly equals: (left: TConfig, right: TConfig) => boolean;
  readonly onListenerError?: (error: unknown) => void | Promise<void>;
}

class DefaultConfigStore<TConfig extends object> implements ConfigStore<TConfig> {
  readonly #load: (signal?: AbortSignal) => Promise<TConfig>;
  readonly #equals: (left: TConfig, right: TConfig) => boolean;
  readonly #onListenerError: ((error: unknown) => void | Promise<void>) | undefined;
  readonly #listeners = new Set<ConfigChangeListener<TConfig>>();

  #current: { readonly value: TConfig } | undefined;
  #status: ConfigStoreStatus = ConfigStoreStatus.Idle;
  #lastError: ConfigError | undefined;
  #inflight: Promise<TConfig> | undefined;
  #disposed = false;

  constructor(internals: StoreInternals<TConfig>) {
    this.#load = internals.load;
    this.#equals = internals.equals;
    this.#onListenerError = internals.onListenerError;
  }

  get status(): ConfigStoreStatus {
    return this.#status;
  }

  get lastError(): ConfigError | undefined {
    return this.#lastError;
  }

  get(): TConfig {
    if (this.#current === undefined) {
      throw new ConfigError('配置存储尚未初始化（内部不变量被破坏）', {
        code: 'internal_error',
      });
    }
    return this.#current.value;
  }

  subscribe(listener: ConfigChangeListener<TConfig>): () => void {
    if (this.#disposed) throw new ConfigDisposedError();
    if (typeof listener !== 'function') {
      throw new ConfigError(`subscribe listener 必须是函数（实际值: ${String(listener)}）`, {
        code: 'invalid_listener',
      });
    }
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  }

  dispose(): void {
    if (this.#disposed) return;
    this.#disposed = true;
    this.#status = ConfigStoreStatus.Disposed;
    this.#listeners.clear();
  }

  reload(signal?: AbortSignal): Promise<TConfig> {
    if (this.#disposed) return Promise.reject(new ConfigDisposedError());
    if (this.#inflight !== undefined) return this.#inflight;

    const inflight = this.#performReload(signal);
    this.#inflight = inflight;
    void (async () => {
      try {
        await inflight;
      } catch {
        // 失败由返回的 inflight Promise 交给调用方；这里只负责清理 single-flight 引用。
      } finally {
        this.#clearInflight(inflight);
      }
    })();
    return inflight;
  }

  async initialize(signal?: AbortSignal): Promise<void> {
    this.#status = ConfigStoreStatus.Loading;
    try {
      const config = await this.#load(signal);
      this.#current = { value: config };
      this.#lastError = undefined;
      this.#status = ConfigStoreStatus.Ready;
    } catch (cause) {
      const error = toConfigError(cause);
      this.#lastError = error;
      this.#status = ConfigStoreStatus.Error;
      throw error;
    }
  }

  async #performReload(signal?: AbortSignal): Promise<TConfig> {
    this.#status = ConfigStoreStatus.Loading;
    let next: TConfig;
    try {
      next = await this.#load(signal);
    } catch (cause) {
      const error = toConfigError(cause);
      this.#lastError = error;
      if (!this.#disposed) this.#status = ConfigStoreStatus.Error;
      throw error;
    }

    const previous = this.#current;
    let hasChanges = false;
    if (!this.#disposed && previous !== undefined) {
      try {
        hasChanges = !this.#equals(previous.value, next);
      } catch (cause) {
        const error = new ConfigError(
          `配置比较失败（previous=${typeof previous.value}, next=${typeof next}）`,
          { code: 'compare_error', cause },
        );
        this.#lastError = error;
        this.#status = ConfigStoreStatus.Error;
        throw error;
      }
    }

    this.#current = { value: next };
    this.#lastError = undefined;
    if (!this.#disposed) this.#status = ConfigStoreStatus.Ready;

    // 首次加载或 dispose 后不再广播；仅当确实发生变化时通知。
    if (!this.#disposed && previous !== undefined && hasChanges) {
      await this.#notify({ config: next, previous: previous.value });
    }
    return next;
  }

  async #notify(change: ConfigChange<TConfig>): Promise<void> {
    const errors: unknown[] = [];
    // 监听器按订阅顺序串行执行：前一个监听器 dispose 后必须阻止后续通知。
    for (const listener of Array.from(this.#listeners)) {
      if (this.#disposed) break;
      try {
        await listener(change);
      } catch (error) {
        if (this.#onListenerError === undefined) {
          errors.push(error);
          continue;
        }
        try {
          await this.#onListenerError(error);
        } catch (handlerError) {
          errors.push(handlerError);
        }
      }
    }
    if (errors.length > 0) {
      throw new AggregateError(errors, '配置变更监听器抛出错误');
    }
  }

  #clearInflight(inflight: Promise<TConfig>): void {
    if (this.#inflight === inflight) this.#inflight = undefined;
  }
}

function toConfigError(cause: unknown): ConfigError {
  if (cause instanceof ConfigError) return cause;
  return new ConfigError('配置加载失败', { code: 'load_error', cause });
}
