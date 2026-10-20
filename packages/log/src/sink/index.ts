/**
 * @file 内置 sink 公共入口。
 * @description 仅做桶导出，不承载业务逻辑。
 * @module log/sink/index
 */

export { createConsoleSink, type ConsoleLike, type ConsoleSinkOptions } from './console.js';
export { createMemorySink, type MemoryLogSink } from './memory.js';
