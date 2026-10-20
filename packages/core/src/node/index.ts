/**
 * @file `@kurumi/core/node` 子路径入口。
 * @description 仅导出依赖 Node 内置模块的实现。
 * @module core/node
 */

export { createNodeRuntimeHost, type NodeRuntimeHostOptions } from './node-host.js';
