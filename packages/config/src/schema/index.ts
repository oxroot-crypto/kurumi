/**
 * @file schema 公共入口。
 * @description 配置包对 `@kurumi/schema` 的适配层；仅做桶导出，不承载业务逻辑。
 * @module config/schema
 */

export { array, object, record, union } from './composites.js';
export type {
  ArraySchemaOptions,
  ConfigShape,
  InferObject,
  InferShape,
  ObjectSchemaOptions,
  RecordSchemaOptions,
  UnknownKeysPolicy,
} from './composites.js';
export { boolean, literal, number, oneOf, string } from './scalars.js';
export type {
  BooleanSchemaOptions,
  LiteralOptions,
  LiteralValue,
  NumberSchemaOptions,
  StringSchemaOptions,
} from './scalars.js';
export {
  Schema,
  SchemaKind,
  isSchema,
  parseFailure,
  parseSuccess,
  toJsonSchema,
} from './schema.js';
export type {
  ConfigSchema,
  Infer,
  JsonSchema,
  ParseFailure,
  ParseOutcome,
  ParseSuccess,
  SchemaKind as SchemaKindType,
  SchemaMetadata,
} from './schema.js';
