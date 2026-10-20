/**
 * @file 任意值到 JSON 安全结构的转换，以及错误对象序列化。
 * @description 公共桶入口；实现拆分到 serialize/ 子模块，保持既有导入路径稳定。
 * @module core/serialize
 */

export * from './serialize/index.js';
