/**
 * @file `@kurumi/sec-edgar` 主入口。
 * @description SEC EDGAR 公开数据只读适配器：参考数据、公司资料与财务报表。
 * @module sec-edgar
 */

export { createSecEdgarFundamentalProvider } from './fundamentals.js';
export { createSecEdgarReferenceProvider } from './reference.js';
export type { SecCompanyTicker, SecEdgarProviderOptions } from './support.js';
