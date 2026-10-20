/**
 * @file 数组、记录与联合 schema 实现。
 * @description 仅承载组合 schema 的解析逻辑；对象实现位于 object-schema.ts。
 * @module schema/composite-schemas
 */

import { isForbiddenKey, isPlainObject } from '@kurumi/core';

import { SchemaDefinitionError } from './errors.js';
import {
  SchemaIssueCode,
  REDACTED_VALUE,
  childPath,
  createIssue,
  describeType,
  type SchemaIssue,
} from './issue.js';
import type { JsonSchema } from './json-schema.js';
import {
  Schema,
  SchemaKind,
  parseFailure,
  parseSuccess,
  type Infer,
  type ParseFailure,
  type ParseOutcome,
} from './schema.js';
import {
  assertDepth,
  assertNonNegativeIntegerOption,
  assertOrderedBounds,
  createMetadata,
  forbiddenKeyIssues,
  invalidTypeFailure,
  isMoreSpecificFailure,
  normalizePattern,
  requiredFailure,
  sizeIssues,
} from './support.js';
import type { ArraySchemaOptions, RecordSchemaOptions } from './composites.js';

/** union 至少需要的分支数量。 */
const MIN_UNION_BRANCHES = 2;

/** 数组 schema 实现。 */
export class ArraySchema<TItem> extends Schema<readonly TItem[]> {
  readonly kind = SchemaKind.Array;
  readonly #itemSchema: Schema<TItem>;
  readonly #minLength: number | undefined;
  readonly #maxLength: number | undefined;

  constructor(itemSchema: Schema<TItem>, options: ArraySchemaOptions) {
    super(createMetadata());
    assertNonNegativeIntegerOption('minLength', options.minLength);
    assertNonNegativeIntegerOption('maxLength', options.maxLength);
    assertOrderedBounds('array 长度', options.minLength, options.maxLength);
    this.#itemSchema = itemSchema;
    this.#minLength = options.minLength;
    this.#maxLength = options.maxLength;
  }

  _parse(value: unknown, path: string, depth: number): ParseOutcome<readonly TItem[]> {
    const depthIssue = assertDepth(depth, path);
    if (depthIssue !== undefined) return depthIssue;
    if (value === undefined) return requiredFailure(path, 'array');
    if (!Array.isArray(value)) return invalidTypeFailure(path, 'array', value);

    const issues: SchemaIssue[] = [];
    issues.push(
      ...sizeIssues(value.length, path, '数组长度', {
        min: this.#minLength,
        max: this.#maxLength,
      }),
    );

    const result: TItem[] = [];
    result.length = value.length;
    for (let index = 0; index < value.length; index += 1) {
      if (!Object.hasOwn(value, index)) {
        issues.push(
          createIssue(
            SchemaIssueCode.InvalidValue,
            childPath(path, index),
            '数组存在空槽（sparse array）',
          ),
        );
        continue;
      }
      const outcome = this.#itemSchema._parse(value[index], childPath(path, index), depth + 1);
      if (outcome.ok) {
        result[index] = outcome.value;
      } else {
        issues.push(...outcome.issues);
      }
    }

    if (issues.length > 0) return parseFailure(issues);
    return parseSuccess(result);
  }

  _redact(value: readonly TItem[]): unknown {
    return value.map((item) => this.#itemSchema._redact(item));
  }

  _toJsonSchema(): JsonSchema {
    return {
      type: 'array',
      items: this.#itemSchema.toJsonSchema(),
      ...(this.#minLength === undefined ? {} : { minItems: this.#minLength }),
      ...(this.#maxLength === undefined ? {} : { maxItems: this.#maxLength }),
    };
  }

  _acceptsUndefined(): boolean {
    return false;
  }
}

/** 记录 schema 实现。 */
export class RecordSchema<TValue> extends Schema<Readonly<Record<string, TValue>>> {
  readonly kind = SchemaKind.Record;
  readonly #valueSchema: Schema<TValue>;
  readonly #keyPattern: RegExp | undefined;
  readonly #keyPatternMessage: string | undefined;
  readonly #minKeys: number | undefined;
  readonly #maxKeys: number | undefined;

  constructor(valueSchema: Schema<TValue>, options: RecordSchemaOptions) {
    super(createMetadata());
    assertNonNegativeIntegerOption('minKeys', options.minKeys);
    assertNonNegativeIntegerOption('maxKeys', options.maxKeys);
    assertOrderedBounds('record 键数量', options.minKeys, options.maxKeys);
    this.#valueSchema = valueSchema;
    this.#keyPattern =
      options.keyPattern === undefined ? undefined : normalizePattern(options.keyPattern);
    this.#keyPatternMessage = options.keyPatternMessage;
    this.#minKeys = options.minKeys;
    this.#maxKeys = options.maxKeys;
  }

  _parse(
    value: unknown,
    path: string,
    depth: number,
  ): ParseOutcome<Readonly<Record<string, TValue>>> {
    const depthIssue = assertDepth(depth, path);
    if (depthIssue !== undefined) return depthIssue;
    if (value === undefined) return requiredFailure(path, 'object');
    if (!isPlainObject(value)) return invalidTypeFailure(path, 'object', value);

    const keys = Object.keys(value);
    const issues: SchemaIssue[] = [];
    issues.push(...forbiddenKeyIssues(keys, path));
    issues.push(
      ...sizeIssues(keys.length, path, '记录键数量', {
        min: this.#minKeys,
        max: this.#maxKeys,
      }),
    );

    const result: Record<string, TValue> = {};
    for (const key of keys) {
      if (isForbiddenKey(key)) continue;
      if (this.#keyPattern !== undefined && !this.#keyPattern.test(key)) {
        issues.push(
          createIssue(
            SchemaIssueCode.PatternMismatch,
            childPath(path, key),
            this.#keyPatternMessage ?? `记录键 "${key}" 不匹配模式 /${this.#keyPattern.source}/`,
            { expected: `/${this.#keyPattern.source}/`, received: key },
          ),
        );
        continue;
      }
      const outcome = this.#valueSchema._parse(value[key], childPath(path, key), depth + 1);
      if (outcome.ok) {
        result[key] = outcome.value;
      } else {
        issues.push(...outcome.issues);
      }
    }

    if (issues.length > 0) return parseFailure(issues);
    return parseSuccess(result);
  }

  _redact(value: Readonly<Record<string, TValue>>): unknown {
    const result: Record<string, unknown> = {};
    for (const [key, item] of Object.entries(value)) {
      if (isForbiddenKey(key)) continue;
      result[key] = this.#valueSchema._redact(item);
    }
    return result;
  }

  _toJsonSchema(): JsonSchema {
    return {
      type: 'object',
      additionalProperties: this.#valueSchema.toJsonSchema(),
      ...(this.#keyPattern === undefined
        ? {}
        : { propertyNames: { pattern: this.#keyPattern.source } }),
      ...(this.#minKeys === undefined ? {} : { minProperties: this.#minKeys }),
      ...(this.#maxKeys === undefined ? {} : { maxProperties: this.#maxKeys }),
    };
  }

  _acceptsUndefined(): boolean {
    return false;
  }
}

/** 联合 schema 实现。 */
export class UnionSchema<TOptions extends readonly Schema<unknown>[]> extends Schema<
  Infer<TOptions[number]>
> {
  readonly kind = SchemaKind.Union;
  readonly #options: readonly Schema<unknown>[];

  constructor(options: TOptions) {
    super(createMetadata());
    if (options.length < MIN_UNION_BRANCHES) {
      throw new SchemaDefinitionError(`union() 至少需要 ${MIN_UNION_BRANCHES} 个分支`);
    }
    this.#options = Object.freeze([...options]);
  }

  _parse(value: unknown, path: string, depth: number): ParseOutcome<Infer<TOptions[number]>> {
    const depthIssue = assertDepth(depth, path);
    if (depthIssue !== undefined) return depthIssue;
    if (value === undefined) return requiredFailure(path, 'union');

    let bestFailure: ParseFailure | undefined;
    for (const option of this.#options) {
      const outcome = option._parse(value, path, depth);
      if (outcome.ok) {
        // 分支 schema 采用 Schema<unknown> 存储，无法让 TS 关联 TOptions 的元素类型；
        // 运行期已由对应分支完成校验。
        return parseSuccess(outcome.value as Infer<TOptions[number]>);
      }
      if (bestFailure === undefined || isMoreSpecificFailure(outcome.issues, bestFailure.issues)) {
        bestFailure = outcome;
      }
    }

    const expected = this.#options.map((option) => option.kind).join(' | ');
    const summary = createIssue(
      SchemaIssueCode.UnionMismatch,
      path,
      `值不匹配任何联合分支（${expected}），实际为 ${describeType(value)}`,
      { expected, received: describeType(value) },
    );
    const relatedIssues = bestFailure?.issues ?? [];
    return parseFailure([summary, ...relatedIssues]);
  }

  _redact(value: Infer<TOptions[number]>): unknown {
    for (const option of this.#options) {
      if (option._parse(value, '$', 0).ok) return option._redact(value);
    }
    return REDACTED_VALUE;
  }

  _toJsonSchema(): JsonSchema {
    return { anyOf: this.#options.map((option) => option.toJsonSchema()) };
  }

  _acceptsUndefined(): boolean {
    return false;
  }
}
