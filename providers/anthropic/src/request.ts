/**
 * @file 将 Agent 模型请求转换为 Anthropic Messages 请求体。
 * @description 只做确定性映射；vendor 扩展通过 providerOptions 透传，核心字段不可被覆盖。
 * @module anthropic/request
 */

import {
  extractMessageText,
  type AgentMessage,
  type ModelRequest,
  type ToolChoice,
} from '@kurumi/agent';
import { isPlainObject } from '@kurumi/core';

import type {
  AnthropicRequestContentBlock,
  AnthropicRequestMessage,
  AnthropicRequestTool,
  AnthropicRequestToolChoice,
  AnthropicRequestToolResultBlock,
} from './dto.js';
import { AnthropicProviderError } from './errors.js';
import type { ResolvedAnthropicProviderOptions } from './options.js';

/** providerOptions 中不得覆盖的核心请求字段。 */
const PROTECTED_BODY_KEYS: ReadonlySet<string> = new Set([
  'model',
  'messages',
  'system',
  'max_tokens',
  'stream',
  'tools',
  'tool_choice',
  'temperature',
]);

/** 消息映射结果。 */
interface MappedMessages {
  /** 顶层 system；没有非空 system 时为 undefined。 */
  readonly system: string | undefined;
  /** user/assistant 消息序列。 */
  readonly messages: readonly AnthropicRequestMessage[];
}

/**
 * 构造请求体。
 *
 * @param request - Agent 模型请求
 * @param options - 解析后的适配器选项
 * @param shouldStream - 是否流式
 * @returns 可直接 JSON 序列化的请求体
 * @throws {AnthropicProviderError} 请求字段、消息结构或工具消息非法
 */
export function toRequestBody(
  request: ModelRequest,
  options: ResolvedAnthropicProviderOptions,
  shouldStream: boolean,
): Record<string, unknown> {
  validateRequest(request, options);
  const extra = isPlainObject(request.providerOptions) ? { ...request.providerOptions } : {};
  for (const key of Object.keys(extra)) {
    if (PROTECTED_BODY_KEYS.has(key)) delete extra[key];
  }

  const mapped = toAnthropicMessages(request.messages, options);
  const body: Record<string, unknown> = {
    ...extra,
    model: request.model,
    max_tokens: request.maxOutputTokens ?? options.defaultMaxTokens,
    messages: mapped.messages,
    stream: shouldStream,
    ...(mapped.system === undefined ? {} : { system: mapped.system }),
    ...(request.temperature === undefined ? {} : { temperature: request.temperature }),
  };

  if (request.tools.length > 0) {
    body['tools'] = request.tools.map(toAnthropicTool);
    if (request.toolChoice !== undefined) {
      body['tool_choice'] = toAnthropicToolChoice(request.toolChoice);
    }
  }
  if (request.thinking !== undefined) {
    body['thinking'] = toAnthropicThinking(request.thinking, options);
  }
  return body;
}

/**
 * 构造请求头。
 *
 * @param options - 解析后的适配器选项
 * @returns content-type、x-api-key、anthropic-version 与自定义头
 */
export function toHeaders(options: ResolvedAnthropicProviderOptions): Record<string, string> {
  const headers: Record<string, string> = {};
  for (const [key, value] of Object.entries(options.headers)) {
    const lower = key.toLowerCase();
    if (lower === 'x-api-key' || lower === 'content-type' || lower === 'anthropic-version') {
      continue;
    }
    headers[key] = value;
  }
  headers['content-type'] = 'application/json';
  headers['x-api-key'] = options.apiKey;
  headers['anthropic-version'] = options.anthropicVersion;
  return headers;
}

/** 校验请求的可变字段。 */
function validateRequest(request: ModelRequest, options: ResolvedAnthropicProviderOptions): void {
  if (typeof request !== 'object' || request === null) {
    throw new AnthropicProviderError(options.id, '请求必须是对象');
  }
  if (typeof request.model !== 'string' || request.model.trim() === '') {
    throw new AnthropicProviderError(options.id, 'request.model 必须是非空字符串');
  }
  if (!Array.isArray(request.messages) || request.messages.length === 0) {
    throw new AnthropicProviderError(options.id, 'request.messages 必须是非空数组');
  }
  const maxOutputTokens = request.maxOutputTokens;
  if (
    maxOutputTokens !== undefined &&
    (!Number.isSafeInteger(maxOutputTokens) || maxOutputTokens <= 0)
  ) {
    throw new AnthropicProviderError(options.id, 'maxOutputTokens 必须是正安全整数');
  }
  const temperature = request.temperature;
  if (
    temperature !== undefined &&
    (typeof temperature !== 'number' || !Number.isFinite(temperature))
  ) {
    throw new AnthropicProviderError(options.id, 'temperature 必须是有限数值');
  }
}

/** 转换消息数组；连续 tool 结果合并为一条 user 消息。 */
function toAnthropicMessages(
  messages: readonly AgentMessage[],
  options: ResolvedAnthropicProviderOptions,
): MappedMessages {
  const systemTexts: string[] = [];
  const converted: AnthropicRequestMessage[] = [];
  let pendingToolResults: AnthropicRequestToolResultBlock[] = [];

  const flushToolResults = (): void => {
    if (pendingToolResults.length === 0) return;
    converted.push({ role: 'user', content: pendingToolResults });
    pendingToolResults = [];
  };

  for (const message of messages) {
    switch (message.role) {
      case 'system':
        flushToolResults();
        systemTexts.push(extractMessageText(message));
        break;
      case 'user':
        flushToolResults();
        converted.push({ role: 'user', content: toUserContent(message, options) });
        break;
      case 'assistant':
        flushToolResults();
        converted.push({ role: 'assistant', content: toAssistantContent(message, options) });
        break;
      case 'tool': {
        const part = message.content[0];
        if (part?.type !== 'tool_result') {
          throw new AnthropicProviderError(options.id, '收到非法 tool 消息');
        }
        pendingToolResults.push({
          type: 'tool_result',
          tool_use_id: part.callId,
          content: part.content,
          ...(part.isError ? { is_error: true } : {}),
        });
        break;
      }
      default:
        throw new AnthropicProviderError(options.id, `不支持的消息角色 ${String(message.role)}`);
    }
  }
  flushToolResults();

  if (converted.length === 0) {
    throw new AnthropicProviderError(options.id, '消息中至少需要一条 user 或 assistant 消息');
  }

  const system = systemTexts.filter((text) => text !== '').join('\n\n');
  return { system: system === '' ? undefined : system, messages: converted };
}

/** 转换 user 内容：纯文本使用字符串，含图片时使用内容数组。 */
function toUserContent(
  message: AgentMessage,
  options: ResolvedAnthropicProviderOptions,
): string | readonly AnthropicRequestContentBlock[] {
  const hasImage = message.content.some((part) => part.type === 'image');
  if (!hasImage) return extractMessageText(message);

  const blocks: AnthropicRequestContentBlock[] = [];
  for (const part of message.content) {
    if (part.type === 'text') {
      if (part.text !== '') blocks.push({ type: 'text', text: part.text });
    } else if (part.type === 'image') {
      blocks.push({
        type: 'image',
        source: { type: 'base64', media_type: part.mediaType, data: part.data },
      });
    } else {
      throw new AnthropicProviderError(options.id, `user 消息包含不支持的 ${part.type} 内容块`);
    }
  }
  return blocks;
}

/** 转换 assistant 内容：reasoning 不回传，text/tool_call 映射为协议块。 */
function toAssistantContent(
  message: AgentMessage,
  options: ResolvedAnthropicProviderOptions,
): readonly AnthropicRequestContentBlock[] {
  const blocks: AnthropicRequestContentBlock[] = [];
  for (const part of message.content) {
    switch (part.type) {
      case 'text':
        blocks.push({ type: 'text', text: part.text });
        break;
      case 'tool_call':
        blocks.push({
          type: 'tool_use',
          id: part.callId,
          name: part.name,
          input: part.input,
        });
        break;
      case 'reasoning':
        if (part.redactedData !== undefined) {
          blocks.push({ type: 'redacted_thinking', data: part.redactedData });
        } else if (part.signature !== undefined) {
          blocks.push({ type: 'thinking', thinking: part.text, signature: part.signature });
        }
        // 缺少签名/加密数据时无法安全回传，跳过以兼容旧消息。
        break;
      default:
        throw new AnthropicProviderError(
          options.id,
          `assistant 消息包含不支持的 ${part.type} 内容块`,
        );
    }
  }
  if (blocks.length === 0) {
    throw new AnthropicProviderError(options.id, 'assistant 消息没有可映射内容');
  }
  return blocks;
}

/** 转换思考配置。 */
function toAnthropicThinking(
  thinking: NonNullable<ModelRequest['thinking']>,
  options: ResolvedAnthropicProviderOptions,
): Record<string, unknown> {
  if (!thinking.isEnabled) return { type: 'disabled' };
  return {
    type: 'enabled',
    budget_tokens: thinking.budgetTokens ?? options.defaultThinkingBudgetTokens,
  };
}

/** 转换工具描述。 */
function toAnthropicTool(tool: ModelRequest['tools'][number]): AnthropicRequestTool {
  return {
    name: tool.name,
    description: tool.description,
    input_schema: tool.inputSchema,
  };
}

/** 转换工具选择。 */
function toAnthropicToolChoice(choice: ToolChoice): AnthropicRequestToolChoice {
  if (typeof choice === 'object') return { type: 'tool', name: choice.name };
  switch (choice) {
    case 'auto':
      return { type: 'auto' };
    case 'none':
      return { type: 'none' };
    case 'required':
      return { type: 'any' };
  }
}
