/**
 * @file OpenAI Chat Completions 请求/响应 DTO。
 * @description 仅描述协议字段，不做运行期校验；校验由 options/request/response 模块完成。
 * @module open-ai/dto
 */

/** 请求内容块（文本或图片）。 */
export type OpenAIRequestContentPart =
  | { readonly type: 'text'; readonly text: string }
  | { readonly type: 'image_url'; readonly image_url: { readonly url: string } };

/** 请求工具调用。 */
export interface OpenAIRequestToolCall {
  readonly id: string;
  readonly type: 'function';
  readonly function: { readonly name: string; readonly arguments: string };
}

/** 请求消息。 */
export interface OpenAIRequestMessage {
  readonly role: 'system' | 'user' | 'assistant' | 'tool';
  readonly content: string | readonly OpenAIRequestContentPart[] | null;
  readonly tool_calls?: readonly OpenAIRequestToolCall[];
  readonly tool_call_id?: string;
  readonly name?: string;
}

/** 请求工具定义。 */
export interface OpenAIRequestTool {
  readonly type: 'function';
  readonly function: {
    readonly name: string;
    readonly description: string;
    readonly parameters: unknown;
  };
}

/** 响应工具调用。 */
export interface OpenAIResponseToolCall {
  readonly id?: string;
  readonly type?: string;
  readonly function?: { readonly name?: string; readonly arguments?: string };
}

/** 响应 assistant 消息。 */
export interface OpenAIResponseMessage {
  readonly content?: string | null;
  readonly tool_calls?: readonly OpenAIResponseToolCall[];
  readonly [key: string]: unknown;
}

/** 响应 choice。 */
export interface OpenAIChoice {
  readonly message?: OpenAIResponseMessage;
  readonly delta?: OpenAIStreamDelta;
  readonly finish_reason?: string | null;
}

/** 响应 usage。 */
export interface OpenAIUsage {
  readonly prompt_tokens?: number;
  readonly completion_tokens?: number;
  readonly total_tokens?: number;
  readonly prompt_tokens_details?: { readonly cached_tokens?: number };
  readonly prompt_cache_hit_tokens?: number;
}

/** 响应 chunk。 */
export interface OpenAIChunk {
  readonly choices?: readonly OpenAIChoice[];
  readonly usage?: OpenAIUsage | null;
  /** 流式错误事件（部分兼容服务会在 200 流中返回）。 */
  readonly error?: unknown;
}

/** 流式工具调用增量。 */
export interface OpenAIStreamToolCallDelta {
  readonly index?: number;
  readonly id?: string;
  readonly type?: string;
  readonly function?: { readonly name?: string; readonly arguments?: string };
}

/** 流式 delta。 */
export interface OpenAIStreamDelta {
  readonly content?: string | null;
  readonly tool_calls?: readonly OpenAIStreamToolCallDelta[];
  readonly [key: string]: unknown;
}
