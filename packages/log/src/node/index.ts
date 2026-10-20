/**
 * @file `@kurumi/log/node` 子路径入口。
 * @description 仅暴露依赖 Node 内置模块的 sink；主入口保持 runtime 无关。
 * @module log/node
 */

export { createNodeFileSink, type NodeFileSink, type NodeFileSinkOptions } from './file-sink.js';
