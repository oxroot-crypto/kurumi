/**
 * @file 配置来源（Source）的公共契约。
 * @description 来源只负责产出一层配置对象；优先级由 loader 按数组顺序决定（后者覆盖前者）。
 * @module config/source
 */

import type { ConfigHost } from './host.js';

/** 来源种类，供诊断与测试使用。 */
export const ConfigSourceKind = Object.freeze({
  /** 读取文件。 */
  File: 'file',
  /** 读取环境变量。 */
  Env: 'env',
  /** 内存对象（测试、CLI 覆盖、默认值注入）。 */
  Object: 'object',
  /** 调用方自定义来源（HTTP、Vault、数据库等）。 */
  Custom: 'custom',
} as const);

/** 来源种类联合类型。 */
export type ConfigSourceKind = (typeof ConfigSourceKind)[keyof typeof ConfigSourceKind];

/** 一层配置对象：键为配置树根字段。 */
export type ConfigLayer = Readonly<Record<string, unknown>>;

/** 来源加载上下文。 */
export interface ConfigSourceContext {
  /** 配置宿主。 */
  readonly host: ConfigHost;
  /** 基准目录，缺省等于 `host.cwd`。 */
  readonly cwd: string;
  /** 可选取消信号；来源应在读取/解析前检查。 */
  readonly signal?: AbortSignal;
}

/** 配置来源。 */
export interface ConfigSource {
  /** 唯一名称；loader 会拒绝重复名称。 */
  readonly name: string;
  /** 来源种类。 */
  readonly kind: ConfigSourceKind;
  /**
   * 加载一层配置。
   *
   * @param context - 宿主与基准目录
   * @returns 配置对象；返回 `undefined` 表示本次来源不提供任何配置
   */
  load(context: ConfigSourceContext): Promise<ConfigLayer | undefined> | ConfigLayer | undefined;
}
