/**
 * @file 将 Agent 模型请求转换为 OpenAI Chat Completions 请求体。
 * @description 只做确定性映射；vendor 扩展通过 providerOptions 透传，核心字段不可被覆盖。
 * @module open-ai/request
 */

import {
  extractMessageText,
  getToolCalls,
  type AgentMessage,
  type ModelRequest,
  type ToolChoice,
} from '@kurumi/agent';
import { isPlainObject } from '@kurumi/core';

import type {
  OpenAIRequestContentPart,
  OpenAIRequestMessage,
  OpenAIRequestTool,
  OpenAIRequestToolCall,
} from './dto.js';
import { OpenAIProviderError } from './errors.js';
import type { ResolvedOpenAIProviderOptions } from './options.js';

/** providerOptions 中不得覆盖的核心请求字段。 */
const PROTECTED_BODY_KEYS: ReadonlySet<string> = new Set([
  'model',
  'messages',
  'stream',
  'stream_options',
  'tools',
  'tool_choice',
  'temperature',
  'max_tokens',
  'max_completion_tokens',
]);

/**
 * 构造请求体。
 *
 * @param request - Agent 模型请求
 * @param options - 解析后的适配器选项
 * @param shouldStream - 是否流式
 * @returns 可直接 JSON 序列化的请求体
 */
export function toRequestBody(
  request: ModelRequest,
  options: ResolvedOpenAIProviderOptions,
  shouldStream: boolean,
): Record<string, unknown> {
  const extra = isPlainObject(request.providerOptions) ? { ...request.providerOptions } : {};
  for (const key of Object.keys(extra)) {
    if (PROTECTED_BODY_KEYS.has(key)) delete extra[key];
  }
  if (request.thinking !== undefined) {
    delete extra['reasoning_effort'];
    if (options.thinkingField !== false) delete extra[options.thinkingField];
  }
  const thinkingToggle =
    request.thinking === undefined || options.thinkingField === false
      ? undefined
      : { [options.thinkingField]: { type: request.thinking.isEnabled ? 'enabled' : 'disabled' } };
  const reasoningEffort =
    request.thinking?.effort === undefined ? undefined : request.thinking.effort;
  const body: Record<string, unknown> = {
    ...extra,
    model: request.model,
    messages: toOpenAIMessages(request.messages, options),
    stream: shouldStream,
    ...(shouldStream && options.shouldIncludeStreamUsage
      ? { stream_options: { include_usage: true } }
      : {}),
    ...(request.temperature === undefined ? {} : { temperature: request.temperature }),
    ...(request.maxOutputTokens === undefined
      ? {}
      : { [options.maxTokensField]: request.maxOutputTokens }),
    ...(reasoningEffort === undefined ? {} : { reasoning_effort: reasoningEffort }),
    ...(thinkingToggle === undefined ? {} : thinkingToggle),
  };

  if (request.tools.length > 0) {
    body['tools'] = request.tools.map(toOpenAITool);
    if (request.toolChoice !== undefined) {
      body['tool_choice'] = toOpenAIToolChoice(request.toolChoice);
    }
  }
  return body;
}

/**
 * 构造请求头。
 *
 * @param options - 解析后的适配器选项
 * @returns content-type、authorization 与自定义头
 */
export function toHeaders(options: ResolvedOpenAIProviderOptions): Record<string, string> {
  const headers: Record<string, string> = {};
  for (const [key, value] of Object.entries(options.headers)) {
    const lower = key.toLowerCase();
    if (lower === 'authorization' || lower === 'content-type') continue;
    headers[key] = value;
  }
  headers['content-type'] = 'application/json';
  headers['authorization'] = `Bearer ${options.apiKey}`;
  return headers;
}

/** 转换消息数组。 */
function toOpenAIMessages(
  messages: readonly AgentMessage[],
  options: ResolvedOpenAIProviderOptions,
): OpenAIRequestMessage[] {
  return messages.map((message): OpenAIRequestMessage => {
    switch (message.role) {
      case 'system':
        return { role: 'system', content: extractMessageText(message) };
      case 'user':
        return { role: 'user', content: toUserContent(message) };
      case 'assistant': {
        const text = extractMessageText(message);
        const calls = getToolCalls(message);
        return {
          role: 'assistant',
          content: text === '' && calls.length > 0 ? null : text,
          ...(calls.length === 0 ? {} : { tool_calls: calls.map(toOpenAIToolCall) }),
        };
      }
      case 'tool': {
        const part = message.content[0];
        if (part?.type !== 'tool_result') {
          throw new OpenAIProviderError(options.id, '收到非法 tool 消息');
        }
        return {
          role: 'tool',
          content: part.content,
          tool_call_id: part.callId,
          name: part.name,
        };
      }
      default:
        throw new OpenAIProviderError(options.id, `不支持的消息角色 ${String(message.role)}`);
    }
  });
}

/** 转换 user 内容：纯文本使用字符串，含图片时使用内容数组。 */
function toUserContent(message: AgentMessage): string | readonly OpenAIRequestContentPart[] {
  const hasImage = message.content.some((part) => part.type === 'image');
  if (!hasImage) return extractMessageText(message);

  const parts: OpenAIRequestContentPart[] = [];
  for (const part of message.content) {
    if (part.type === 'text') {
      if (part.text !== '') parts.push({ type: 'text', text: part.text });
    } else if (part.type === 'image') {
      parts.push({
        type: 'image_url',
        image_url: { url: `data:${part.mediaType};base64,${part.data}` },
      });
    }
  }
  return parts;
}

/** 转换工具调用。 */
function toOpenAIToolCall(call: ReturnType<typeof getToolCalls>[number]): OpenAIRequestToolCall {
  return {
    id: call.callId,
    type: 'function',
    function: { name: call.name, arguments: JSON.stringify(call.input) },
  };
}

/** 转换工具描述。 */
function toOpenAITool(tool: ModelRequest['tools'][number]): OpenAIRequestTool {
  return {
    type: 'function',
    function: {
      name: tool.name,
      description: tool.description,
      parameters: tool.inputSchema,
    },
  };
}

/** 转换工具选择。 */
function toOpenAIToolChoice(choice: ToolChoice): unknown {
  if (typeof choice === 'object') {
    return { type: 'function', function: { name: choice.name } };
  }
  return choice;
}
