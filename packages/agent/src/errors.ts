/**
 * @file Agent 领域错误。
 * @description 复用 core 的 KurumiError 语义，区分定义、消息、模型与工具契约失败。
 * @module agent/errors
 */

import { KurumiError, type KurumiErrorOptions } from '@kurumi/core';

/** Agent 包所有错误的基类。 */
export class AgentError extends KurumiError {
  /**
   * @param message - 面向人类的错误描述
   * @param options - 错误码与 cause
   */
  constructor(message: string, options?: KurumiErrorOptions) {
    super(message, { code: 'agent_error', ...options });
  }
}

/** Agent/工具/运行选项定义不合法。 */
export class AgentDefinitionError extends AgentError {
  /**
   * @param message - 非法的定义或选项描述
   * @param cause - 底层校验错误
   */
  constructor(message: string, cause?: unknown) {
    super(message, {
      code: 'agent_definition_error',
      ...(cause === undefined ? {} : { cause }),
    });
  }
}

/** 消息结构不符合角色/内容矩阵。 */
export class AgentMessageError extends AgentError {
  /** 出错的数据路径（如 `$.messages[0].content[1]`）。 */
  readonly path: string;

  /**
   * @param message - 消息结构错误描述
   * @param options - 出错路径与底层 cause
   */
  constructor(message: string, options?: { readonly path?: string; readonly cause?: unknown }) {
    super(message, {
      code: 'agent_message_error',
      ...(options?.cause === undefined ? {} : { cause: options.cause }),
    });
    this.path = options?.path ?? '$';
  }
}

/** 模型适配器调用失败或返回了违反协议的数据。 */
export class AgentModelError extends AgentError {
  /** 出错的模型提供方 ID。 */
  readonly providerId: string;

  /**
   * @param providerId - 模型提供方 ID
   * @param message - 失败描述
   * @param cause - 适配器抛出的原始错误或消息校验错误
   */
  constructor(providerId: string, message: string, cause?: unknown) {
    super(`模型提供方 "${providerId}" ${message}`, {
      code: 'agent_model_error',
      ...(cause === undefined ? {} : { cause }),
    });
    this.providerId = providerId;
  }
}

/** 工具实现返回了不符合 ToolResult 契约的值。 */
export class AgentToolError extends AgentError {
  /** 出错的工具名称。 */
  readonly toolName: string;

  /**
   * @param toolName - 工具名称
   * @param message - 失败描述
   * @param cause - 底层校验错误
   */
  constructor(toolName: string, message: string, cause?: unknown) {
    super(`工具 "${toolName}" ${message}`, {
      code: 'agent_tool_error',
      ...(cause === undefined ? {} : { cause }),
    });
    this.toolName = toolName;
  }
}
