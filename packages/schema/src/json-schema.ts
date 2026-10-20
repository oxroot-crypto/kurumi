/**
 * @file JSON Schema 类型别名。
 * @description 导出的 JSON Schema 必然是 JSON 安全值，可直接交给模型提供方。
 * @module schema/json-schema
 */

import type { JsonValue } from '@kurumi/core';

/** JSON Schema 对象；值必须是 JSON 安全值。 */
export type JsonSchema = { readonly [key: string]: JsonValue };
