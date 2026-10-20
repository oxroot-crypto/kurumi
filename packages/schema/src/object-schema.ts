/**
 * @file 对象 schema 实现。
 * @description 处理字段校验、未知键策略、危险键与 section 缺失策略。
 * @module schema/object-schema
 */

import { cloneJsonValue, isForbiddenKey, isPlainObject } from '@kurumi/core';

import { SchemaDefinitionError } from './errors.js';
import {
  SchemaIssueCode,
  childPath,
  createIssue,
  describeType,
  type SchemaIssue,
} from './issue.js';
import type { JsonSchema } from './json-schema.js';
import { Schema, SchemaKind, parseFailure, parseSuccess, type ParseOutcome } from './schema.js';
import {
  assertDepth,
  createMetadata,
  forbiddenKeyIssues,
  invalidTypeFailure,
  requiredFailure,
} from './support.js';
import type {
  InferObject,
  ObjectSchemaOptions,
  SchemaShape,
  UnknownKeysPolicy,
} from './composites.js';

/** 对象 schema 实现。 */
export class ObjectSchema<
  TShape extends SchemaShape,
  TOptions extends ObjectSchemaOptions,
> extends Schema<InferObject<TShape, TOptions>> {
  readonly kind = SchemaKind.Object;
  readonly #shape: SchemaShape;
  readonly #unknownKeys: UnknownKeysPolicy;
  readonly #missingSection: 'empty' | 'error';

  constructor(shape: TShape, options: ObjectSchemaOptions) {
    super(createMetadata());
    for (const key of Object.keys(shape)) {
      if (isForbiddenKey(key)) {
        throw new SchemaDefinitionError(`对象 schema 不允许危险字段名 "${key}"`);
      }
    }
    const unknownKeys = options.unknownKeys ?? 'error';
    if (unknownKeys !== 'error' && unknownKeys !== 'strip' && unknownKeys !== 'passthrough') {
      throw new SchemaDefinitionError(`未知的 unknownKeys 策略: ${String(unknownKeys)}`);
    }
    const missingSection = options.missingSection ?? 'error';
    if (missingSection !== 'empty' && missingSection !== 'error') {
      throw new SchemaDefinitionError(`未知的 missingSection 策略: ${String(missingSection)}`);
    }
    // shape 是 schema 的一部分，必须与外部引用脱钩，否则构造后仍可改变解析行为。
    this.#shape = Object.freeze({ ...shape });
    this.#unknownKeys = unknownKeys;
    this.#missingSection = missingSection;
  }

  _parse(value: unknown, path: string, depth: number): ParseOutcome<InferObject<TShape, TOptions>> {
    const depthIssue = assertDepth(depth, path);
    if (depthIssue !== undefined) return depthIssue;
    // 对象 section 缺失默认按错误处理；配置系统会显式改为 empty 以启用嵌套默认值。
    if (value === undefined) {
      if (this.#missingSection === 'error') return requiredFailure(path, 'object');
      value = {};
    }
    if (!isPlainObject(value)) return invalidTypeFailure(path, 'object', value);

    const issues: SchemaIssue[] = [];
    const result: Record<string, unknown> = {};

    issues.push(...forbiddenKeyIssues(Object.keys(value), path));

    for (const key of Object.keys(this.#shape)) {
      const childSchema = this.#shape[key];
      if (childSchema === undefined) continue;
      const outcome = childSchema._parse(value[key], childPath(path, key), depth + 1);
      if (outcome.ok) {
        result[key] = outcome.value;
      } else {
        issues.push(...outcome.issues);
      }
    }

    for (const key of Object.keys(value)) {
      if (Object.hasOwn(this.#shape, key) || isForbiddenKey(key)) continue;
      if (this.#unknownKeys === 'error') {
        issues.push(
          createIssue(SchemaIssueCode.UnknownKey, childPath(path, key), `未知 schema 键 "${key}"`, {
            expected: `one of: ${Object.keys(this.#shape).join(', ') || '(none)'}`,
          }),
        );
      } else if (this.#unknownKeys === 'passthrough') {
        const unknownValue = value[key];
        if (unknownValue === undefined) continue;
        try {
          // passthrough 只保留 JSON 安全值，并克隆以切断与调用方输入的引用共享。
          result[key] = cloneJsonValue(unknownValue, childPath(path, key));
        } catch {
          issues.push(
            createIssue(
              SchemaIssueCode.InvalidValue,
              childPath(path, key),
              'passthrough 未知键的值必须是 JSON 安全值',
              { expected: 'JSON-safe value', received: describeType(unknownValue) },
            ),
          );
        }
      }
    }

    if (issues.length > 0) return parseFailure(issues);
    // 动态 shape 无法被 TypeScript 精确推导；字段已逐个通过 childSchema._parse 校验。
    return parseSuccess(result as InferObject<TShape, TOptions>);
  }

  _redact(value: InferObject<TShape, TOptions>): unknown {
    const recordValue: Record<string, unknown> = { ...value };
    const result: Record<string, unknown> = {};
    for (const key of Object.keys(this.#shape)) {
      const childSchema = this.#shape[key];
      if (childSchema === undefined || !Object.hasOwn(recordValue, key)) continue;
      result[key] = childSchema._redact(recordValue[key]);
    }
    if (this.#unknownKeys === 'passthrough') {
      for (const key of Object.keys(recordValue)) {
        if (!Object.hasOwn(this.#shape, key) && !isForbiddenKey(key)) {
          result[key] = recordValue[key];
        }
      }
    }
    return result;
  }

  _toJsonSchema(): JsonSchema {
    const properties: Record<string, JsonSchema> = {};
    const required: string[] = [];
    for (const [key, childSchema] of Object.entries(this.#shape)) {
      properties[key] = childSchema.toJsonSchema();
      if (!childSchema._acceptsUndefined()) required.push(key);
    }
    return {
      type: 'object',
      properties,
      ...(required.length === 0 ? {} : { required }),
      // strip/passthrough 都允许调用方传入额外键；解析阶段再决定保留或丢弃。
      additionalProperties: this.#unknownKeys === 'error' ? false : true,
    };
  }

  _acceptsUndefined(): boolean {
    // missingSection: 'empty' 时 undefined 会被当作 {} 解析；只有所有字段也接受缺失才会成功。
    return (
      this.#missingSection === 'empty' &&
      Object.values(this.#shape).every((childSchema) => childSchema._acceptsUndefined())
    );
  }
}
