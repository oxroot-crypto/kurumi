/**
 * @file 配置宿主类型别名。
 * @description 宿主抽象已下沉到 core；config 仅保留领域命名，避免调用方感知包结构。
 * @module config/host
 */

export type { RuntimeHost as ConfigHost } from '@kurumi/core';
