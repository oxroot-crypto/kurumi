/**
 * @file 环境变量配置来源。
 * @description 将 `PREFIX` 开头的变量映射为嵌套对象；`__` 分隔层级，纯数字段自动成为数组下标。
 * @module config/source/env-source
 */

import { MAX_JSON_DEPTH, isForbiddenKey, isPlainObject } from '@kurumi/core';

import { ConfigEnvMappingError, ConfigSchemaDefinitionError } from '../errors.js';
import { ConfigIssueCode, childPath, createIssue, type ConfigIssue } from '../issue.js';
import {
  ConfigSourceKind,
  type ConfigLayer,
  type ConfigSource,
  type ConfigSourceContext,
} from '../source.js';

/** 环境变量数组下标上限，防止超长数组导致内存放大。 */
const MAX_ENV_ARRAY_INDEX = 10_000;

/** 环境变量来源选项。 */
export interface EnvSourceOptions {
  /** 变量名前缀（例如 `KURUMI_`），必须匹配 `[A-Za-z_][A-Za-z0-9_]*`。 */
  readonly prefix: string;
  /** 来源唯一名称，缺省 `env:<prefix>`。 */
  readonly name?: string;
  /** 层级分隔符，缺省 `__`；必须由非字母数字、非空白字符组成。 */
  readonly separator?: string;
  /** 是否将路径段转为小写，缺省 true。 */
  readonly shouldLowercase?: boolean;
  /** 是否把 `{...}` / `[...]` 形式的变量值解析为 JSON，缺省 true。 */
  readonly shouldParseStructuredValues?: boolean;
}

type PathSegment =
  | { readonly kind: 'key'; readonly value: string }
  | { readonly kind: 'index'; readonly value: number };

/**
 * 创建环境变量配置来源。
 *
 * @remarks
 * 示例（prefix `KURUMI_`、separator `__`）：
 * - `KURUMI_SERVER__PORT=3000` → `{ server: { port: '3000' } }`（由 schema 强转为 number）
 * - `KURUMI_CORS__ORIGINS__0=https://a` → `{ cors: { origins: ['https://a'] } }`
 * - `KURUMI_CORS__ORIGINS=["a","b"]` → 直接得到数组
 *
 * @param options - 前缀、分隔符与大小写策略
 * @returns 配置来源
 * @throws {ConfigSchemaDefinitionError} 前缀/分隔符/名称非法
 */
export function envSource(options: EnvSourceOptions): ConfigSource {
  if (typeof options !== 'object' || options === null) {
    throw new ConfigSchemaDefinitionError('envSource options 必须是对象');
  }
  const prefix = options.prefix;
  if (typeof prefix !== 'string' || !/^[A-Za-z_][A-Za-z0-9_]*$/.test(prefix)) {
    throw new ConfigSchemaDefinitionError(
      `envSource prefix 必须匹配 [A-Za-z_][A-Za-z0-9_]*，实际为 "${String(prefix)}"`,
    );
  }
  const separator = options.separator ?? '__';
  if (typeof separator !== 'string' || !/^[^A-Za-z0-9\s]+$/.test(separator)) {
    throw new ConfigSchemaDefinitionError(
      `envSource separator 必须由非字母数字、非空白字符组成，实际为 "${String(separator)}"`,
    );
  }
  if (options.name !== undefined && typeof options.name !== 'string') {
    throw new ConfigSchemaDefinitionError(
      `envSource name 必须是字符串（实际值: ${String(options.name)}）`,
    );
  }
  assertBooleanOption('shouldLowercase', options.shouldLowercase);
  assertBooleanOption('shouldParseStructuredValues', options.shouldParseStructuredValues);
  const name = options.name ?? `env:${prefix}`;
  if (name.trim() === '') {
    throw new ConfigSchemaDefinitionError('envSource name 不能为空');
  }

  return new EnvSource({
    name,
    prefix,
    separator,
    shouldLowercase: options.shouldLowercase ?? true,
    shouldParseStructuredValues: options.shouldParseStructuredValues ?? true,
  });
}

/** 校验布尔选项。 */
function assertBooleanOption(name: string, value: boolean | undefined): void {
  if (value !== undefined && typeof value !== 'boolean') {
    throw new ConfigSchemaDefinitionError(`${name} 必须是布尔值，实际为 ${String(value)}`);
  }
}

interface ResolvedEnvSourceOptions {
  readonly name: string;
  readonly prefix: string;
  readonly separator: string;
  readonly shouldLowercase: boolean;
  readonly shouldParseStructuredValues: boolean;
}

class EnvSource implements ConfigSource {
  readonly name: string;
  readonly kind = ConfigSourceKind.Env;
  readonly #prefix: string;
  readonly #separator: string;
  readonly #shouldLowercase: boolean;
  readonly #shouldParseStructuredValues: boolean;

  constructor(options: ResolvedEnvSourceOptions) {
    this.name = options.name;
    this.#prefix = options.prefix;
    this.#separator = options.separator;
    this.#shouldLowercase = options.shouldLowercase;
    this.#shouldParseStructuredValues = options.shouldParseStructuredValues;
  }

  load(context: ConfigSourceContext): ConfigLayer {
    context.signal?.throwIfAborted();
    const env = context.host.env();
    const issues: ConfigIssue[] = [];
    const root: Record<string, unknown> = {};
    this.#collectEntries(env, root, issues);

    validateArrayContiguity(root, '$', issues);
    if (issues.length > 0) throw new ConfigEnvMappingError(this.name, issues);
    return root;
  }

  #collectEntries(
    env: Readonly<Record<string, string | undefined>>,
    root: Record<string, unknown>,
    issues: ConfigIssue[],
  ): void {
    for (const key of Object.keys(env).sort()) {
      if (!key.startsWith(this.#prefix)) continue;
      const pathSegments = this.#parsePath(key, issues);
      if (pathSegments === undefined) continue;
      const rawValue = env[key];
      if (rawValue === undefined) continue;
      setPath(root, pathSegments, this.#parseValue(rawValue), { issues, envKey: key });
    }
  }

  /** 将环境变量名解析为路径段；非法路径会写入 issues 并返回 undefined。 */
  #parsePath(envKey: string, issues: ConfigIssue[]): readonly PathSegment[] | undefined {
    const rest = envKey.slice(this.#prefix.length);
    if (rest === '') return undefined;

    const rawSegments = rest.split(this.#separator);
    if (rawSegments.some((segment) => segment === '')) {
      issues.push(
        createIssue(
          ConfigIssueCode.EnvConflict,
          '$',
          `环境变量 "${envKey}" 包含空路径段（检查分隔符 "${this.#separator}"）`,
        ),
      );
      return undefined;
    }

    const segments = rawSegments.map((segment) => this.#toSegment(segment, envKey, issues));
    const pathSegments = segments.filter(
      (segment): segment is PathSegment => segment !== undefined,
    );
    if (pathSegments.length !== segments.length) return undefined;
    const first = pathSegments[0];
    if (first === undefined) return undefined;
    if (first.kind === 'index') {
      issues.push(
        createIssue(
          ConfigIssueCode.InvalidRoot,
          '$',
          `环境变量 "${envKey}" 映射到数组根，配置根必须是对象`,
        ),
      );
      return undefined;
    }
    if (pathSegments.some((segment) => segment.kind === 'key' && isForbiddenKey(segment.value))) {
      issues.push(
        createIssue(ConfigIssueCode.ForbiddenKey, '$', `环境变量 "${envKey}" 映射出危险键`),
      );
      return undefined;
    }
    return pathSegments;
  }

  #toSegment(segment: string, envKey: string, issues: ConfigIssue[]): PathSegment | undefined {
    const normalized = this.#shouldLowercase ? segment.toLowerCase() : segment;
    if (/^(?:0|[1-9]\d*)$/.test(normalized)) {
      const index = Number(normalized);
      if (!Number.isSafeInteger(index) || index > MAX_ENV_ARRAY_INDEX) {
        issues.push(
          createIssue(
            ConfigIssueCode.EnvIndexGap,
            '$',
            `环境变量 "${envKey}" 的数组下标 ${normalized} 超过上限 ${MAX_ENV_ARRAY_INDEX}`,
          ),
        );
        return undefined;
      }
      return { kind: 'index', value: index };
    }
    return { kind: 'key', value: normalized };
  }

  #parseValue(raw: string): unknown {
    if (!this.#shouldParseStructuredValues) return raw;
    const trimmed = raw.trim();
    if (!trimmed.startsWith('{') && !trimmed.startsWith('[')) return raw;
    try {
      return JSON.parse(trimmed);
    } catch {
      // 不是合法 JSON 时按普通字符串处理，避免把以 '[' 开头的普通文本误判为错误。
      return raw;
    }
  }
}

type Container = Record<string, unknown> | unknown[];

interface PathContext {
  readonly issues: ConfigIssue[];
  readonly envKey: string;
}

interface PathStepBase {
  readonly container: Container;
  readonly next: PathSegment | undefined;
  readonly isLast: boolean;
  readonly path: string;
  readonly context: PathContext;
}

interface IndexPathStep extends PathStepBase {
  readonly kind: 'index';
  readonly index: number;
}

interface KeyPathStep extends PathStepBase {
  readonly kind: 'key';
  readonly key: string;
}

type StepResult =
  | { readonly kind: 'next'; readonly container: Container }
  | { readonly kind: 'done' }
  | { readonly kind: 'conflict' };

function setPath(
  root: Record<string, unknown>,
  segments: readonly PathSegment[],
  value: unknown,
  context: PathContext,
): void {
  let current: Container = root;
  let path = '$';

  for (let index = 0; index < segments.length; index += 1) {
    const segment = segments[index];
    if (segment === undefined) return;
    path = childPath(path, segment.value);
    const common: PathStepBase = {
      container: current,
      next: segments[index + 1],
      isLast: index === segments.length - 1,
      path,
      context,
    };
    const step: IndexPathStep | KeyPathStep =
      segment.kind === 'index'
        ? { ...common, kind: 'index', index: segment.value }
        : { ...common, kind: 'key', key: segment.value };
    const result: StepResult =
      step.kind === 'index' ? applyIndexStep(step, value) : applyKeyStep(step, value);

    if (result.kind === 'conflict') {
      conflict(path, context);
      return;
    }
    if (result.kind === 'done') return;
    current = result.container;
  }
}

function applyIndexStep(step: IndexPathStep, value: unknown): StepResult {
  if (!Array.isArray(step.container)) return { kind: 'conflict' };
  const current = step.container;

  if (step.isLast) {
    if (Object.hasOwn(current, step.index)) return { kind: 'conflict' };
    current[step.index] = value;
    return { kind: 'done' };
  }

  const existing: unknown = current[step.index];
  if (existing === undefined) {
    const container = step.next?.kind === 'index' ? [] : {};
    current[step.index] = container;
    return { kind: 'next', container };
  }
  if (!isCompatibleContainer(existing, step.next)) return { kind: 'conflict' };
  return { kind: 'next', container: existing };
}

function applyKeyStep(step: KeyPathStep, value: unknown): StepResult {
  if (!isPlainObject(step.container)) return { kind: 'conflict' };
  const current = step.container;

  if (step.isLast) {
    if (Object.hasOwn(current, step.key)) return { kind: 'conflict' };
    current[step.key] = value;
    return { kind: 'done' };
  }

  const existing: unknown = current[step.key];
  if (existing === undefined) {
    const container = step.next?.kind === 'index' ? [] : {};
    current[step.key] = container;
    return { kind: 'next', container };
  }
  if (!isCompatibleContainer(existing, step.next)) return { kind: 'conflict' };
  return { kind: 'next', container: existing };
}

function isCompatibleContainer(
  value: unknown,
  next: PathSegment | undefined,
): value is Record<string, unknown> | unknown[] {
  return next?.kind === 'index' ? Array.isArray(value) : isPlainObject(value);
}

function conflict(path: string, context: PathContext): void {
  context.issues.push(
    createIssue(
      ConfigIssueCode.EnvConflict,
      path,
      `环境变量 "${context.envKey}" 与已有路径冲突（当前值不是所需的对象/数组）`,
    ),
  );
}

function validateArrayContiguity(
  value: unknown,
  path: string,
  issues: ConfigIssue[],
  depth = 0,
): void {
  if (depth > MAX_JSON_DEPTH) {
    issues.push(
      createIssue(
        ConfigIssueCode.MaxDepth,
        path,
        `环境变量结构化值的嵌套深度超过上限 ${MAX_JSON_DEPTH}`,
        { expected: `depth <= ${MAX_JSON_DEPTH}`, received: `depth ${depth}` },
      ),
    );
    return;
  }

  if (Array.isArray(value)) {
    for (let index = 0; index < value.length; index += 1) {
      if (!Object.hasOwn(value, index)) {
        issues.push(
          createIssue(
            ConfigIssueCode.EnvIndexGap,
            childPath(path, index),
            `环境变量数组索引不连续，缺少下标 ${index}`,
          ),
        );
        continue;
      }
      validateArrayContiguity(value[index], childPath(path, index), issues, depth + 1);
    }
    return;
  }
  if (isPlainObject(value)) {
    for (const key of Object.keys(value)) {
      if (isForbiddenKey(key)) {
        issues.push(
          createIssue(
            ConfigIssueCode.ForbiddenKey,
            childPath(path, key),
            `环境变量结构化值中出现危险键 "${key}"`,
          ),
        );
        continue;
      }
      validateArrayContiguity(value[key], childPath(path, key), issues, depth + 1);
    }
  }
}
