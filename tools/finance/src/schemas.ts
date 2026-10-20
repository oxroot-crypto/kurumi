/**
 * @file 金融 Agent 工具共享 schema。
 * @description 所有输入在工具边界由 `@kurumi/schema` 校验；每个字段都带 describe，供模型理解参数含义。
 * @module finance-tools/schemas
 */

import { array, number, object, oneOf, string } from '@kurumi/schema';

/** 标的 key。 */
export const INSTRUMENT_KEY_SCHEMA = string({ isNonEmpty: true }).describe(
  '标的唯一 key，格式为 venue:symbol，例如 binance:BTC/USDT、nasdaq:AAPL、fx:EUR/USD',
);

/** 货币代码。 */
export const CURRENCY_CODE_SCHEMA = string({ pattern: /^[A-Z][A-Z0-9]{2,11}$/ }).describe(
  '大写货币/资产代码，例如 USD、EUR、USDT、BTC',
);

/** 十进制字符串。 */
export const DECIMAL_STRING_SCHEMA = string({
  // 与 Decimal.parse 的语法保持一致：不接受 `1.`，允许 `.5` 与科学计数法。
  pattern: /^[+-]?(?:\d+(?:\.\d+)?|\.\d+)(?:[eE][+-]?\d+)?$/,
}).describe('十进制数值字符串（禁止用浮点数传金额/价格），例如 "123.45"；支持科学计数法');

/** `YYYY-MM-DD` 日期。 */
export const DATE_SCHEMA = string({ pattern: /^\d{4}-\d{2}-\d{2}$/ }).describe(
  'UTC 日期，格式 YYYY-MM-DD，例如 "2026-10-19"',
);

/** epoch 毫秒。 */
export const TIMESTAMP_MS_SCHEMA = number({ isInteger: true, min: 0 }).describe(
  'Unix epoch 毫秒时间戳，例如 1792368000000',
);

/** 正安全整数。 */
export const POSITIVE_INTEGER_SCHEMA = number({ isInteger: true, min: 1 }).describe(
  '正安全整数（>= 1）',
);

/** 非负安全整数。 */
export const NON_NEGATIVE_INTEGER_SCHEMA = number({ isInteger: true, min: 0 }).describe(
  '非负安全整数（>= 0）',
);

/** 期权方向。 */
export const OPTION_TYPE_SCHEMA = oneOf(['call', 'put'] as const).describe(
  '期权方向：call 表示看涨期权，put 表示看跌期权',
);

/** 买卖方向。 */
export const SIDE_SCHEMA = oneOf(['buy', 'sell'] as const).describe(
  '买卖方向：buy 买入/做多，sell 卖出/做空',
);

/** K 线周期。 */
export const INTERVAL_SCHEMA = oneOf([
  '1m',
  '3m',
  '5m',
  '15m',
  '30m',
  '1h',
  '2h',
  '4h',
  '6h',
  '8h',
  '12h',
  '1d',
  '3d',
  '1w',
] as const).describe('K 线周期：m=分钟、h=小时、d=天、w=周，例如 "1h"');

/** 资产类别。 */
export const ASSET_CLASS_SCHEMA = oneOf([
  'equity',
  'crypto',
  'fx',
  'future',
  'perpetual',
  'option',
  'index',
] as const).describe(
  '资产类别：equity 股票、crypto 加密货币、fx 外汇、future 期货、perpetual 永续、option 期权、index 指数',
);

/** 报表期间类型。 */
export const PERIOD_TYPE_SCHEMA = oneOf(['annual', 'quarterly', 'ttm'] as const).describe(
  '财务报表期间：annual 年报、quarterly 季报、ttm 滚动十二个月',
);

/** 金额输入。 */
export const MONEY_SCHEMA = object({
  amount: DECIMAL_STRING_SCHEMA.describe('金额数值的十进制字符串，例如 "100.50"'),
  currency: CURRENCY_CODE_SCHEMA.describe('金额币种代码，例如 USD、USDT'),
}).describe('金额对象：十进制数值 + 货币代码');

/** OHLCV 点。 */
export const OHLCV_POINT_SCHEMA = object({
  timestampMs: TIMESTAMP_MS_SCHEMA.describe('该 K 线的开盘时间（epoch 毫秒）'),
  open: number().describe('开盘价'),
  high: number().describe('最高价，必须 >= open/close/low'),
  low: number().describe('最低价，必须 <= open/close/high'),
  close: number().describe('收盘价'),
  volume: number({ min: 0 }).describe('成交量，必须 >= 0'),
}).describe('单根 OHLCV K 线');

/** 标的价格/数量点。 */
export const PRICE_POINT_SCHEMA = object({
  timestampMs: TIMESTAMP_MS_SCHEMA.describe('价格对应的时间（epoch 毫秒）'),
  price: DECIMAL_STRING_SCHEMA.describe('该时点的价格十进制字符串'),
}).describe('历史价格点');

/** 成交输入。 */
export const FILL_INPUT_SCHEMA = object({
  side: SIDE_SCHEMA.describe('成交方向：buy 买入，sell 卖出'),
  price: DECIMAL_STRING_SCHEMA.describe('成交价十进制字符串'),
  quantity: DECIMAL_STRING_SCHEMA.describe('成交量十进制字符串，必须 > 0'),
  timestampMs: TIMESTAMP_MS_SCHEMA.describe('成交时间（epoch 毫秒），同一序列必须非降序'),
  fee: MONEY_SCHEMA.describe('可选手续费金额；币种必须等于标的计价货币').optional(),
}).describe('单笔成交');

/** 标的 key 数组输入。 */
export const INSTRUMENT_LIST_SCHEMA = array(INSTRUMENT_KEY_SCHEMA, { maxLength: 100 }).describe(
  '标的 key 列表，用于白名单/限制',
);
