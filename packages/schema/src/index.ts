/**
 * @file `@kurumi/schema` 主入口。
 * @description 通用 schema 定义、校验、敏感值遮蔽与 JSON Schema 导出；桶导出，不承载业务逻辑。
 * @module schema
 */

export { array, object, record, union } from './composites.js';
export type {
  ArraySchemaOptions,
  InferObject,
  InferShape,
  ObjectSchemaOptions,
  RecordSchemaOptions,
  SchemaShape,
  UnknownKeysPolicy,
} from './composites.js';
export {
  SchemaDefinitionError,
  SchemaError,
  SchemaValidationError,
  type SchemaErrorOptions,
} from './errors.js';
export {
  REDACTED_VALUE,
  ROOT_PATH,
  SchemaIssueCode,
  childPath,
  createIssue,
  describeType,
  displayPath,
  formatIssues,
  type SchemaIssue,
} from './issue.js';
export type { JsonSchema } from './json-schema.js';
export {
  Schema,
  SchemaKind,
  isSchema,
  parseFailure,
  parseSuccess,
  toJsonSchema,
} from './schema.js';
export type {
  Infer,
  ParseFailure,
  ParseOutcome,
  ParseSuccess,
  SchemaKind as SchemaKindType,
  SchemaMetadata,
} from './schema.js';
export { boolean, literal, number, oneOf, string } from './scalars.js';
export type {
  BooleanSchemaOptions,
  LiteralOptions,
  LiteralValue,
  NumberSchemaOptions,
  StringSchemaOptions,
} from './scalars.js';
