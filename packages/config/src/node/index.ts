/**
 * @file `@kurumi/config/node` 子路径入口。
 * @description Node 宿主实现已下沉到 core，这里保留稳定的配置包导出名称。
 * @module config/node
 */

export {
  createNodeRuntimeHost as createNodeConfigHost,
  type NodeRuntimeHostOptions as NodeConfigHostOptions,
} from '@kurumi/core/node';
