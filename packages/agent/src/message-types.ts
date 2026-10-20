/**
 * @file Agent 消息、内容块与工具消息输入的类型契约。
 * @description 只包含数据形状；运行期校验与构造位于 message.ts。
 * @module agent/message-types
 */

import type { JsonValue } from '@kurumi/core';

/** 消息角色。 */
export const AgentMessageRole = {
  System: 'system',
  User: 'user',
  Assistant: 'assistant',
  Tool: 'tool',
} as const;

/** 消息角色联合类型。 */
export type AgentMessageRole = (typeof AgentMessageRole)[keyof typeof AgentMessageRole];

/** 文本内容块。 */
export interface TextContentPart {
  /** 内容块判别字段。 */
  readonly type: 'text';
  /** 文本内容。 */
  readonly text: string;
}

/** 图片内容块；图片以 base64 编码传入。 */
export interface ImageContentPart {
  /** 内容块判别字段。 */
  readonly type: 'image';
  /** MIME 类型，例如 `image/png`。 */
  readonly mediaType: string;
  /** base64 编码的图片数据。 */
  readonly data: string;
}

/** 模型的推理/思考内容块；不参与最终文本拼接。 */
export interface ReasoningContentPart {
  /** 内容块判别字段。 */
  readonly type: 'reasoning';
  /** 推理文本。 */
  readonly text: string;
  /** 提供方不透明签名；用于在后续请求中原样回传思考块。 */
  readonly signature?: string;
  /** 提供方加密的思考数据；存在时优先于文本/签名回传。 */
  readonly redactedData?: string;
}

/** 模型发起的工具调用。 */
export interface ToolCallContentPart {
  /** 内容块判别字段。 */
  readonly type: 'tool_call';
  /** 调用 ID，由模型提供，消息内必须唯一。 */
  readonly callId: string;
  /** 工具名称。 */
  readonly name: string;
  /** JSON 安全的工具输入。 */
  readonly input: JsonValue;
}

/** 工具执行结果。 */
export interface ToolResultContentPart {
  /** 内容块判别字段。 */
  readonly type: 'tool_result';
  /** 被响应的调用 ID。 */
  readonly callId: string;
  /** 工具名称。 */
  readonly name: string;
  /** 面向模型的文本结果。 */
  readonly content: string;
  /** 工具是否以失败结束。 */
  readonly isError: boolean;
  /** 可选的 JSON 安全元数据，供上层程序消费。 */
  readonly metadata?: JsonValue;
}

/** 消息内容块联合类型。 */
export type AgentContentPart =
  | TextContentPart
  | ImageContentPart
  | ReasoningContentPart
  | ToolCallContentPart
  | ToolResultContentPart;

/** 一条不可变的 Agent 消息。 */
export interface AgentMessage {
  /** 消息角色。 */
  readonly role: AgentMessageRole;
  /** 至少一个内容块。 */
  readonly content: readonly AgentContentPart[];
}

/** 创建消息时允许的简写内容。 */
export type AgentMessageContent = string | readonly AgentContentPart[];

/** `createToolMessage` 的输入参数。 */
export interface CreateToolMessageInput {
  /** 被响应的调用 ID，非空字符串。 */
  readonly callId: string;
  /** 工具名称。 */
  readonly name: string;
  /** 面向模型的文本结果。 */
  readonly content: string;
  /** 工具是否以失败结束，缺省 false。 */
  readonly isError?: boolean;
  /** 可选的 JSON 安全元数据。 */
  readonly metadata?: JsonValue;
}
