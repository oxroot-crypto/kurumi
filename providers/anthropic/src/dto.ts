/**
 * @file Anthropic Messages 请求/响应 DTO。
 * @description 仅描述协议字段，不做运行期校验；校验由 options/request/response 模块完成。
 * @module anthropic/dto
 */

/** 请求图片内容块。 */
export interface AnthropicRequestImageBlock {
  readonly type: 'image';
  readonly source: {
    readonly type: 'base64';
    readonly media_type: string;
    readonly data: string;
  };
}

/** 请求文本内容块。 */
export interface AnthropicRequestTextBlock {
  readonly type: 'text';
  readonly text: string;
}

/** 请求工具调用内容块。 */
export interface AnthropicRequestToolUseBlock {
  readonly type: 'tool_use';
  readonly id: string;
  readonly name: string;
  readonly input: unknown;
}

/** 请求工具结果内容块。 */
export interface AnthropicRequestToolResultBlock {
  readonly type: 'tool_result';
  readonly tool_use_id: string;
  readonly content: string;
  readonly is_error?: boolean;
}

/** 请求思考内容块；仅用于回传带签名的历史 thinking。 */
export interface AnthropicRequestThinkingBlock {
  readonly type: 'thinking';
  readonly thinking: string;
  readonly signature: string;
}

/** 请求加密思考内容块；仅用于回传 redacted_thinking。 */
export interface AnthropicRequestRedactedThinkingBlock {
  readonly type: 'redacted_thinking';
  readonly data: string;
}

/** 请求内容块联合类型。 */
export type AnthropicRequestContentBlock =
  | AnthropicRequestImageBlock
  | AnthropicRequestTextBlock
  | AnthropicRequestToolUseBlock
  | AnthropicRequestToolResultBlock
  | AnthropicRequestThinkingBlock
  | AnthropicRequestRedactedThinkingBlock;

/** 请求消息；Anthropic Messages API 只接受 user/assistant。 */
export interface AnthropicRequestMessage {
  readonly role: 'user' | 'assistant';
  readonly content: string | readonly AnthropicRequestContentBlock[];
}

/** 请求工具定义。 */
export interface AnthropicRequestTool {
  readonly name: string;
  readonly description: string;
  readonly input_schema: unknown;
}

/** 请求工具选择。 */
export type AnthropicRequestToolChoice =
  | { readonly type: 'auto' }
  | { readonly type: 'none' }
  | { readonly type: 'any' }
  | { readonly type: 'tool'; readonly name: string };

/** 响应 usage。 */
export interface AnthropicUsage {
  readonly input_tokens?: number;
  readonly output_tokens?: number;
  readonly cache_creation_input_tokens?: number;
  readonly cache_read_input_tokens?: number;
  readonly [key: string]: unknown;
}

/** 响应文本内容块。 */
export interface AnthropicResponseTextBlock {
  readonly type: 'text';
  readonly text?: unknown;
  readonly [key: string]: unknown;
}

/** 响应思考内容块。 */
export interface AnthropicResponseThinkingBlock {
  readonly type: 'thinking';
  readonly thinking?: unknown;
  readonly signature?: unknown;
  readonly [key: string]: unknown;
}

/** 响应加密思考内容块。 */
export interface AnthropicResponseRedactedThinkingBlock {
  readonly type: 'redacted_thinking';
  readonly data?: unknown;
  readonly [key: string]: unknown;
}

/** 响应工具调用内容块。 */
export interface AnthropicResponseToolUseBlock {
  readonly type: 'tool_use';
  readonly id?: unknown;
  readonly name?: unknown;
  readonly input?: unknown;
  readonly [key: string]: unknown;
}

/** 响应内容块联合类型（含未知类型，由 response 模块显式失败）。 */
export type AnthropicResponseContentBlock =
  | AnthropicResponseTextBlock
  | AnthropicResponseThinkingBlock
  | AnthropicResponseRedactedThinkingBlock
  | AnthropicResponseToolUseBlock
  | { readonly type: string; readonly [key: string]: unknown };

/** 非流式响应。 */
export interface AnthropicResponse {
  readonly content?: unknown;
  readonly role?: unknown;
  readonly type?: unknown;
  readonly stop_reason?: unknown;
  readonly stop_sequence?: unknown;
  readonly usage?: AnthropicUsage | null;
  readonly [key: string]: unknown;
}

/** 流事件。 */
export interface AnthropicStreamEvent {
  readonly type?: unknown;
  readonly index?: unknown;
  readonly message?: unknown;
  readonly content_block?: unknown;
  readonly delta?: unknown;
  readonly usage?: unknown;
  readonly error?: unknown;
  readonly [key: string]: unknown;
}
