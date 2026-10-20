/**
 * @file 标量 schema 实现类。
 * @description 仅承载 string/number/boolean/literal 的解析逻辑；公共工厂位于 scalars.ts。
 * @module schema/scalar-schemas
 */

import { SchemaDefinitionError } from './errors.js';
import { SchemaIssueCode, createIssue } from './issue.js';
import type { JsonSchema } from './json-schema.js';
import { Schema, SchemaKind, parseFailure, parseSuccess, type ParseOutcome } from './schema.js';
import {
  assertFiniteNumberOption,
  assertOrderedBounds,
  codePointLength,
  createMetadata,
  describeValue,
  invalidTypeFailure,
  normalizePattern,
  requiredFailure,
} from './support.js';
import type {
  BooleanSchemaOptions,
  LiteralValue,
  NumberSchemaOptions,
  StringSchemaOptions,
} from './scalars.js';

/** 数字字符串的严格语法：拒绝十六进制、下划线、空串、Infinity/NaN。 */
export const NUMERIC_STRING_PATTERN = /^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$/;

/** 字符串 schema 实现。 */
export class StringSchema extends Schema<string> {
  readonly kind = SchemaKind.String;
  readonly #minLength: number | undefined;
  readonly #maxLength: number | undefined;
  readonly #pattern: RegExp | undefined;
  readonly #patternMessage: string | undefined;
  readonly #shouldTrim: boolean;
  readonly #isNonEmpty: boolean;

  constructor(options: StringSchemaOptions) {
    super(createMetadata());
    assertNonNegativeLength('minLength', options.minLength);
    assertNonNegativeLength('maxLength', options.maxLength);
    assertOrderedBounds('string 长度', options.minLength, options.maxLength);
    assertBooleanOption('shouldTrim', options.shouldTrim);
    assertBooleanOption('isNonEmpty', options.isNonEmpty);
    this.#minLength =
      options.isNonEmpty === true ? Math.max(options.minLength ?? 0, 1) : options.minLength;
    this.#maxLength = options.maxLength;
    this.#pattern = options.pattern === undefined ? undefined : normalizePattern(options.pattern);
    this.#patternMessage = options.patternMessage;
    this.#shouldTrim = options.shouldTrim ?? false;
    this.#isNonEmpty = options.isNonEmpty === true;
  }

  _parse(value: unknown, path: string, _depth: number): ParseOutcome<string> {
    if (value === undefined) return requiredFailure(path, 'string');
    if (typeof value !== 'string') return invalidTypeFailure(path, 'string', value);

    const normalized = this.#shouldTrim ? value.trim() : value;
    // isNonEmpty 的定义是“trim 后非空”，不能用 minLength 代替；否则 shouldTrim=false 时纯空白会漏过。
    const checked = this.#isNonEmpty ? normalized.trim() : normalized;
    const length = codePointLength(checked);
    if (this.#minLength !== undefined && length < this.#minLength) {
      return parseFailure([
        createIssue(
          SchemaIssueCode.TooSmall,
          path,
          `字符串长度 ${length} 小于下限 ${this.#minLength}`,
          { expected: `length >= ${this.#minLength}`, received: `length ${length}` },
        ),
      ]);
    }
    if (this.#maxLength !== undefined && length > this.#maxLength) {
      return parseFailure([
        createIssue(
          SchemaIssueCode.TooLarge,
          path,
          `字符串长度 ${length} 大于上限 ${this.#maxLength}`,
          { expected: `length <= ${this.#maxLength}`, received: `length ${length}` },
        ),
      ]);
    }
    if (this.#pattern !== undefined && !this.#pattern.test(normalized)) {
      return parseFailure([
        createIssue(
          SchemaIssueCode.PatternMismatch,
          path,
          this.#patternMessage ?? `字符串不匹配模式 /${this.#pattern.source}/`,
          {
            expected: `/${this.#pattern.source}/`,
            received: normalized,
          },
        ),
      ]);
    }
    return parseSuccess(normalized);
  }

  _redact(value: string): unknown {
    return value;
  }

  _toJsonSchema(): JsonSchema {
    return {
      type: 'string',
      ...(this.#minLength === undefined ? {} : { minLength: this.#minLength }),
      ...(this.#maxLength === undefined ? {} : { maxLength: this.#maxLength }),
      ...(this.#pattern === undefined ? {} : { pattern: this.#pattern.source }),
    };
  }

  _acceptsUndefined(): boolean {
    return false;
  }
}

/** 数值 schema 实现。 */
export class NumberSchema extends Schema<number> {
  readonly kind = SchemaKind.Number;
  readonly #min: number | undefined;
  readonly #max: number | undefined;
  readonly #isInteger: boolean;
  readonly #allowNonFinite: boolean;
  readonly #shouldCoerceStrings: boolean;

  constructor(options: NumberSchemaOptions) {
    super(createMetadata());
    assertFiniteNumberOption('min', options.min);
    assertFiniteNumberOption('max', options.max);
    assertOrderedBounds('number 范围', options.min, options.max);
    assertBooleanOption('isInteger', options.isInteger);
    assertBooleanOption('allowNonFinite', options.allowNonFinite);
    assertBooleanOption('shouldCoerceStrings', options.shouldCoerceStrings);
    this.#min = options.min;
    this.#max = options.max;
    this.#isInteger = options.isInteger ?? false;
    this.#allowNonFinite = options.allowNonFinite ?? false;
    this.#shouldCoerceStrings = options.shouldCoerceStrings ?? false;
  }

  _parse(value: unknown, path: string, _depth: number): ParseOutcome<number> {
    if (value === undefined) return requiredFailure(path, 'number');
    const numeric = this.#readNumber(value);
    if (numeric === undefined) return invalidTypeFailure(path, 'number', value);

    if (Number.isNaN(numeric) || (!this.#allowNonFinite && !Number.isFinite(numeric))) {
      return parseFailure([
        createIssue(
          SchemaIssueCode.NotFinite,
          path,
          `数值必须是有限数，实际为 ${String(numeric)}`,
          {
            expected: this.#allowNonFinite ? 'number (非 NaN)' : 'finite number',
            received: String(numeric),
          },
        ),
      ]);
    }
    if (this.#isInteger && !Number.isInteger(numeric)) {
      return parseFailure([
        createIssue(SchemaIssueCode.NotInteger, path, `期望整数，实际为 ${numeric}`, {
          expected: 'integer',
          received: String(numeric),
        }),
      ]);
    }
    if (this.#min !== undefined && numeric < this.#min) {
      return parseFailure([
        createIssue(SchemaIssueCode.TooSmall, path, `数值 ${numeric} 小于下限 ${this.#min}`, {
          expected: `>= ${this.#min}`,
          received: String(numeric),
        }),
      ]);
    }
    if (this.#max !== undefined && numeric > this.#max) {
      return parseFailure([
        createIssue(SchemaIssueCode.TooLarge, path, `数值 ${numeric} 大于上限 ${this.#max}`, {
          expected: `<= ${this.#max}`,
          received: String(numeric),
        }),
      ]);
    }
    return parseSuccess(numeric);
  }

  _redact(value: number): unknown {
    return value;
  }

  _toJsonSchema(): JsonSchema {
    return {
      type: this.#isInteger ? 'integer' : 'number',
      ...(this.#min === undefined ? {} : { minimum: this.#min }),
      ...(this.#max === undefined ? {} : { maximum: this.#max }),
    };
  }

  _acceptsUndefined(): boolean {
    return false;
  }

  #readNumber(value: unknown): number | undefined {
    if (typeof value === 'number') return value;
    if (!this.#shouldCoerceStrings || typeof value !== 'string') return undefined;
    const trimmed = value.trim();
    if (!NUMERIC_STRING_PATTERN.test(trimmed)) return undefined;
    return Number(trimmed);
  }
}

/** 布尔 schema 实现。 */
export class BooleanSchema extends Schema<boolean> {
  readonly kind = SchemaKind.Boolean;
  readonly #shouldCoerceStrings: boolean;

  constructor(options: BooleanSchemaOptions) {
    super(createMetadata());
    assertBooleanOption('shouldCoerceStrings', options.shouldCoerceStrings);
    this.#shouldCoerceStrings = options.shouldCoerceStrings ?? false;
  }

  _parse(value: unknown, path: string, _depth: number): ParseOutcome<boolean> {
    if (value === undefined) return requiredFailure(path, 'boolean');
    if (typeof value === 'boolean') return parseSuccess(value);
    if (this.#shouldCoerceStrings && typeof value === 'string') {
      const coerced = parseBooleanString(value);
      if (coerced !== undefined) return parseSuccess(coerced);
    }
    return invalidTypeFailure(path, 'boolean', value);
  }

  _redact(value: boolean): unknown {
    return value;
  }

  _toJsonSchema(): JsonSchema {
    return { type: 'boolean' };
  }

  _acceptsUndefined(): boolean {
    return false;
  }
}

/** 单字面量 schema 实现。 */
export class LiteralSchema<TValue extends LiteralValue> extends Schema<TValue> {
  readonly kind = SchemaKind.Literal;
  readonly #expected: TValue;
  readonly #shouldCoerceStrings: boolean;

  constructor(expected: TValue, shouldCoerceStrings: boolean) {
    super(createMetadata());
    this.#expected = expected;
    this.#shouldCoerceStrings = shouldCoerceStrings;
  }

  _parse(value: unknown, path: string, _depth: number): ParseOutcome<TValue> {
    if (value === undefined) return requiredFailure(path, 'literal');
    if (!matchesLiteral(value, this.#expected, this.#shouldCoerceStrings)) {
      return parseFailure([
        createIssue(
          SchemaIssueCode.InvalidLiteral,
          path,
          `期望字面量 ${JSON.stringify(this.#expected)}，实际为 ${describeValue(value)}`,
          { expected: JSON.stringify(this.#expected), received: describeValue(value) },
        ),
      ]);
    }
    return parseSuccess(this.#expected);
  }

  _redact(value: TValue): unknown {
    return value;
  }

  _toJsonSchema(): JsonSchema {
    return { const: this.#expected };
  }

  _acceptsUndefined(): boolean {
    return false;
  }
}

/** 字面量集合 schema 实现。 */
export class LiteralSetSchema<TValues extends readonly LiteralValue[]> extends Schema<
  TValues[number]
> {
  readonly kind = SchemaKind.Literal;
  readonly #values: readonly LiteralValue[];
  readonly #shouldCoerceStrings: boolean;

  constructor(values: TValues, shouldCoerceStrings: boolean) {
    super(createMetadata());
    this.#values = Object.freeze([...values]);
    this.#shouldCoerceStrings = shouldCoerceStrings;
  }

  _parse(value: unknown, path: string, _depth: number): ParseOutcome<TValues[number]> {
    if (value === undefined) return requiredFailure(path, 'literal');
    for (const expected of this.#values) {
      if (matchesLiteral(value, expected, this.#shouldCoerceStrings)) {
        // 值已与集合中的某个字面量匹配；TS 无法把循环变量收窄为 TValues[number]。
        return parseSuccess(expected as TValues[number]);
      }
    }
    const allowed = this.#values.map((item) => JSON.stringify(item)).join(' | ');
    return parseFailure([
      createIssue(SchemaIssueCode.InvalidLiteral, path, `期望 ${allowed} 之一`, {
        expected: allowed,
        received: describeValue(value),
      }),
    ]);
  }

  _redact(value: TValues[number]): unknown {
    return value;
  }

  _toJsonSchema(): JsonSchema {
    return { enum: this.#values };
  }

  _acceptsUndefined(): boolean {
    return false;
  }
}

/** 校验非负整数长度选项。 */
function assertNonNegativeLength(name: string, value: number | undefined): void {
  if (value === undefined) return;
  if (!Number.isInteger(value) || value < 0) {
    throw new SchemaDefinitionError(`${name} 必须是非负整数，实际为 ${String(value)}`);
  }
}

/** 校验布尔选项类型。 */
export function assertBooleanOption(name: string, value: boolean | undefined): void {
  if (value !== undefined && typeof value !== 'boolean') {
    throw new SchemaDefinitionError(`${name} 必须是布尔值，实际为 ${String(value)}`);
  }
}

/** 校验字面量是否为合法 JSON 原始值；有限数值、字符串、布尔或 null。 */
export function assertValidLiteral(value: unknown): asserts value is LiteralValue {
  if (value === null || typeof value === 'string' || typeof value === 'boolean') return;
  if (typeof value === 'number' && Number.isFinite(value)) return;
  throw new SchemaDefinitionError(
    `字面量必须是有限数值、字符串、布尔或 null（实际值: ${String(value)}）`,
  );
}

/** 解析布尔字符串。 */
function parseBooleanString(value: string): boolean | undefined {
  switch (value.trim().toLowerCase()) {
    case 'true':
    case '1':
      return true;
    case 'false':
    case '0':
      return false;
    default:
      return undefined;
  }
}

/** 判断输入是否匹配字面量。 */
function matchesLiteral(
  value: unknown,
  expected: LiteralValue,
  shouldCoerceStrings: boolean,
): boolean {
  if (expected === null) return value === null;
  if (typeof expected === 'string') return value === expected;
  if (typeof expected === 'number') {
    if (typeof value === 'number') return Object.is(value, expected);
    if (
      shouldCoerceStrings &&
      typeof value === 'string' &&
      NUMERIC_STRING_PATTERN.test(value.trim())
    ) {
      return Object.is(Number(value.trim()), expected);
    }
    return false;
  }
  if (typeof value === 'boolean') return value === expected;
  if (shouldCoerceStrings && typeof value === 'string') {
    return parseBooleanString(value) === expected;
  }
  return false;
}
