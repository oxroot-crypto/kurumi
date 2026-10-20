/**
 * @file `@kurumi/finance-tools` 主入口。
 * @description Agent 金融工具：provider 查询工具与 finance/math 分析工具；provider 由调用方注入。
 * @module finance-tools
 */

import { createToolRegistry, type ToolDefinition, type ToolRegistry } from '@kurumi/agent';

import { createFinanceAnalysisTools } from './analysis-tools.js';
import { createFinanceCompositeTools } from './composite-tools.js';
import { createFinanceProviderTools } from './provider-tools.js';
import type { FinanceToolsOptions } from './options.js';

export { createFinanceAnalysisTools } from './analysis-tools.js';
export { createFinanceCompositeTools } from './composite-tools.js';
export { FinanceToolError } from './errors.js';
export { toJsonValue, toolResult } from './json.js';
export { createFinanceProviderTools, parseDateUtc } from './provider-tools.js';
export type {
  FinanceProviderToolOptions,
  FinanceToolContext,
  FinanceToolOptions,
  FinanceToolsOptions,
} from './options.js';
export {
  ASSET_CLASS_SCHEMA,
  CURRENCY_CODE_SCHEMA,
  DATE_SCHEMA,
  DECIMAL_STRING_SCHEMA,
  FILL_INPUT_SCHEMA,
  INSTRUMENT_KEY_SCHEMA,
  INSTRUMENT_LIST_SCHEMA,
  INTERVAL_SCHEMA,
  MONEY_SCHEMA,
  NON_NEGATIVE_INTEGER_SCHEMA,
  OHLCV_POINT_SCHEMA,
  OPTION_TYPE_SCHEMA,
  PERIOD_TYPE_SCHEMA,
  POSITIVE_INTEGER_SCHEMA,
  PRICE_POINT_SCHEMA,
  SIDE_SCHEMA,
  TIMESTAMP_MS_SCHEMA,
} from './schemas.js';

/**
 * 创建全部金融工具。
 *
 * @remarks
 * 默认包含纯分析、复合分析和低层 provider 查询三类工具；低层工具只在对应 provider
 * 能力存在时生成。工具数量较多时，可通过 `shouldIncludeProviderTools: false` 只保留
 * 复合分析工具，避免模型在相似能力之间来回选择。
 *
 * @param options - provider 注入、标的/货币配置与工具分组开关
 * @returns 按“分析 -> 复合 -> provider 查询”顺序排列的工具定义
 */
export function createFinanceTools(options: FinanceToolsOptions = {}): readonly ToolDefinition[] {
  return Object.freeze([
    ...(options.shouldIncludeAnalysisTools === false ? [] : createFinanceAnalysisTools(options)),
    ...(options.shouldIncludeCompositeTools === false ? [] : createFinanceCompositeTools(options)),
    ...(options.shouldIncludeProviderTools === false ? [] : createFinanceProviderTools(options)),
  ]);
}

/**
 * 创建金融工具注册表。
 *
 * @param options - provider 注入、标的/货币配置与工具分组开关
 * @returns Agent `ToolRegistry`
 */
export function createFinanceToolRegistry(options: FinanceToolsOptions = {}): ToolRegistry {
  return createToolRegistry([...createFinanceTools(options)]);
}
