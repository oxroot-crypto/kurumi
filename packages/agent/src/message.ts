/**
 * @file Agent 消息、内容块与运行时校验。
 * @description 消息是模型与工具之间的唯一数据契约；所有输入在此归一化并递归冻结。
 * @module agent/message
 */

import { cloneJsonValue, deepFreeze, isPlainObject, type JsonValue } from '@kurumi/core';

import { AgentMessageError } from './errors.js';
import {
  AgentMessageRole,
  type AgentContentPart,
  type AgentMessage,
  type AgentMessageContent,
  type CreateToolMessageInput,
  type ImageContentPart,
  type ReasoningContentPart,
  type TextContentPart,
  type ToolCallContentPart,
  type ToolResultContentPart,
} from './message-types.js';

export { AgentMessageRole } from './message-types.js';
export type {
  AgentContentPart,
  AgentMessage,
  AgentMessageContent,
  CreateToolMessageInput,
  ImageContentPart,
  ReasoningContentPart,
  TextContentPart,
  ToolCallContentPart,
  ToolResultContentPart,
} from './message-types.js';

/** 工具名称匹配模式：字母开头，最长 64 字符。 */
const TOOL_NAME_PATTERN = /^[A-Za-z][A-Za-z0-9_-]{0,63}$/;

/** 图片 MIME 类型匹配模式。 */
const IMAGE_MEDIA_TYPE_PATTERN = /^image\/[A-Za-z0-9.+-]+$/;

/**
 * 判断工具名称是否符合 Agent 契约。
 *
 * @param value - 待判断值
 * @returns 符合 `^[A-Za-z][A-Za-z0-9_-]{0,63}$` 时返回 true
 */
export function isValidToolName(value: unknown): value is string {
  return typeof value === 'string' && TOOL_NAME_PATTERN.test(value);
}

/**
 * 按角色创建一条不可变消息。
 *
 * @param role - 消息角色
 * @param content - 字符串简写或至少一个内容块
 * @returns 递归冻结的消息
 * @throws {AgentMessageError} 角色/内容组合非法、工具调用输入非 JSON 安全值
 *
 * @example
 * ```ts
 * const message = createMessage(AgentMessageRole.User, '你好');
 * ```
 */
export function createMessage(role: AgentMessageRole, content: AgentMessageContent): AgentMessage {
  return normalizeAgentMessage({ role, content });
}

/**
 * 创建 system 消息。
 *
 * @param content - 系统提示文本
 * @returns 冻结的 system 消息
 * @throws {AgentMessageError} 内容非法
 */
export function createSystemMessage(content: string): AgentMessage {
  return createMessage(AgentMessageRole.System, content);
}

/**
 * 创建 user 消息。
 *
 * @param content - 文本或文本/图片内容块
 * @returns 冻结的 user 消息
 * @throws {AgentMessageError} 内容非法
 */
export function createUserMessage(content: AgentMessageContent): AgentMessage {
  return createMessage(AgentMessageRole.User, content);
}

/**
 * 创建 assistant 消息。
 *
 * @param content - 文本或文本/工具调用内容块
 * @returns 冻结的 assistant 消息
 * @throws {AgentMessageError} 内容非法
 */
export function createAssistantMessage(content: AgentMessageContent): AgentMessage {
  return createMessage(AgentMessageRole.Assistant, content);
}

/**
 * 创建单条工具结果消息。
 *
 * @param input - 调用 ID、工具名、结果文本与可选错误标记
 * @returns 冻结的 tool 消息，内容恰好包含一个 `tool_result` 块
 * @throws {AgentMessageError} 输入字段非法
 *
 * @example
 * ```ts
 * const message = createToolMessage({ callId: 'c1', name: 'search', content: 'ok' });
 * ```
 */
export function createToolMessage(input: CreateToolMessageInput): AgentMessage {
  return createMessage(AgentMessageRole.Tool, [
    {
      type: 'tool_result',
      callId: input.callId,
      name: input.name,
      content: input.content,
      isError: input.isError ?? false,
      ...(input.metadata === undefined ? {} : { metadata: input.metadata }),
    },
  ]);
}

/**
 * 校验并归一化任意消息输入。
 *
 * @param value - 未经信任的消息对象
 * @param path - 出错路径，用于诊断
 * @returns 递归冻结的消息副本
 * @throws {AgentMessageError} 角色、内容块或角色/内容组合非法
 */
export function normalizeAgentMessage(value: unknown, path = '$'): AgentMessage {
  if (!isPlainObject(value)) {
    throw messageError(`${path} 必须是普通对象`, path);
  }
  const role = value['role'];
  if (!isAgentMessageRole(role)) {
    throw messageError(`${path}.role 必须是 system/user/assistant/tool`, path);
  }
  const content = normalizeMessageContent(value['content'], `${path}.content`);
  assertRoleContent(role, content, path);
  assertUniqueToolCallIds(content, path);
  return Object.freeze({ role, content: Object.freeze(content) });
}

/**
 * 提取消息中的全部工具调用。
 *
 * @param message - 已校验的消息
 * @returns 冻结的工具调用块数组；无工具调用时为空数组
 */
export function getToolCalls(message: AgentMessage): readonly ToolCallContentPart[] {
  return Object.freeze(
    message.content.filter((part): part is ToolCallContentPart => part.type === 'tool_call'),
  );
}

/**
 * 拼接消息中的全部文本内容块。
 *
 * @param message - 已校验的消息
 * @returns 文本块按声明顺序直接拼接的结果；无文本块时为空字符串
 */
export function extractMessageText(message: AgentMessage): string {
  return message.content
    .filter((part): part is TextContentPart => part.type === 'text')
    .map((part) => part.text)
    .join('');
}

/**
 * 拼接消息中的全部推理内容块。
 *
 * @param message - 已校验的消息
 * @returns 推理文本按声明顺序直接拼接的结果；无推理块时为空字符串
 */
export function extractMessageReasoning(message: AgentMessage): string {
  return message.content
    .filter((part): part is ReasoningContentPart => part.type === 'reasoning')
    .map((part) => part.text)
    .join('');
}

/** 判断角色是否是合法联合成员。 */
function isAgentMessageRole(value: unknown): value is AgentMessageRole {
  return (
    value === AgentMessageRole.System ||
    value === AgentMessageRole.User ||
    value === AgentMessageRole.Assistant ||
    value === AgentMessageRole.Tool
  );
}

/** 归一化字符串简写或内容块数组。 */
function normalizeMessageContent(value: unknown, path: string): AgentContentPart[] {
  if (typeof value === 'string') return [createTextPart(value)];
  if (!Array.isArray(value) || value.length === 0) {
    throw messageError(`${path} 必须是非空字符串或至少一个内容块`, path);
  }
  return value.map((part, index) => normalizeContentPart(part, `${path}[${index}]`));
}

/** 校验角色与内容块矩阵。 */
function assertRoleContent(
  role: AgentMessageRole,
  content: readonly AgentContentPart[],
  path: string,
): void {
  if (role === AgentMessageRole.Tool) {
    if (content.length !== 1 || content[0]?.type !== 'tool_result') {
      throw messageError(`${path}.content 作为 tool 消息必须恰好包含一个 tool_result`, path);
    }
    return;
  }

  for (const part of content) {
    if (role === AgentMessageRole.System && part.type !== 'text') {
      throw messageError(`${path}.content 作为 system 消息只允许 text`, path);
    }
    if (role === AgentMessageRole.User && part.type !== 'text' && part.type !== 'image') {
      throw messageError(`${path}.content 作为 user 消息只允许 text/image`, path);
    }
    if (
      role === AgentMessageRole.Assistant &&
      part.type !== 'text' &&
      part.type !== 'reasoning' &&
      part.type !== 'tool_call'
    ) {
      throw messageError(
        `${path}.content 作为 assistant 消息只允许 text/reasoning/tool_call`,
        path,
      );
    }
  }
}

/** 校验同一消息内 tool_call 的 callId 唯一。 */
function assertUniqueToolCallIds(content: readonly AgentContentPart[], path: string): void {
  const seen = new Set<string>();
  for (const part of content) {
    if (part.type !== 'tool_call') continue;
    if (seen.has(part.callId)) {
      throw messageError(`${path}.content 包含重复的 callId "${part.callId}"`, path);
    }
    seen.add(part.callId);
  }
}

/** 校验并归一化单个内容块。 */
function normalizeContentPart(value: unknown, path: string): AgentContentPart {
  if (!isPlainObject(value)) {
    throw messageError(`${path} 必须是普通对象`, path);
  }
  switch (value['type']) {
    case 'text':
      return createTextPart(value['text'], path);
    case 'reasoning':
      return createReasoningPart(value, path);
    case 'image':
      return createImagePart(value, path);
    case 'tool_call':
      return createToolCallPart(value, path);
    case 'tool_result':
      return createToolResultPart(value, path);
    default:
      throw messageError(`${path}.type 必须是 text/reasoning/image/tool_call/tool_result`, path);
  }
}

/** 创建文本块。 */
function createTextPart(value: unknown, path?: string): TextContentPart {
  if (typeof value !== 'string') {
    throw messageError(`${path ?? '$'}.text 必须是字符串`, path);
  }
  return Object.freeze({ type: 'text', text: value });
}

/** 创建推理块。 */
function createReasoningPart(value: Record<string, unknown>, path: string): ReasoningContentPart {
  const text = value['text'];
  if (typeof text !== 'string') {
    throw messageError(`${path}.text 必须是字符串`, path);
  }
  const signature = readOptionalContinuation(value['signature'], `${path}.signature`, path);
  const redactedData = readOptionalContinuation(
    value['redactedData'],
    `${path}.redactedData`,
    path,
  );
  if (signature !== undefined && redactedData !== undefined) {
    throw messageError(`${path}.signature 与 redactedData 不能同时存在`, path);
  }
  return Object.freeze({
    type: 'reasoning',
    text,
    ...(signature === undefined ? {} : { signature }),
    ...(redactedData === undefined ? {} : { redactedData }),
  });
}

/** 校验可选的提供方续传字符串。 */
function readOptionalContinuation(value: unknown, label: string, path: string): string | undefined {
  if (value === undefined) return undefined;
  if (typeof value !== 'string' || value.trim() === '') {
    throw messageError(`${label} 必须是非空字符串`, path);
  }
  return value;
}

/** 创建图片块。 */
function createImagePart(value: Record<string, unknown>, path: string): ImageContentPart {
  const mediaType = value['mediaType'];
  const data = value['data'];
  if (typeof mediaType !== 'string' || !IMAGE_MEDIA_TYPE_PATTERN.test(mediaType)) {
    throw messageError(`${path}.mediaType 必须是合法图片 MIME 类型`, path);
  }
  if (typeof data !== 'string' || data.trim() === '') {
    throw messageError(`${path}.data 必须是非空 base64 字符串`, path);
  }
  return Object.freeze({ type: 'image', mediaType, data });
}

/** 创建工具调用块。 */
function createToolCallPart(value: Record<string, unknown>, path: string): ToolCallContentPart {
  const callId = requireNonEmptyString(value['callId'], `${path}.callId`, path);
  const name = requireToolName(value['name'], `${path}.name`, path);
  const input = cloneJsonInput(value['input'], `${path}.input`);
  return Object.freeze({ type: 'tool_call', callId, name, input });
}

/** 创建工具结果块。 */
function createToolResultPart(value: Record<string, unknown>, path: string): ToolResultContentPart {
  const callId = requireNonEmptyString(value['callId'], `${path}.callId`, path);
  const name = requireToolName(value['name'], `${path}.name`, path);
  const content = value['content'];
  if (typeof content !== 'string') {
    throw messageError(`${path}.content 必须是字符串`, path);
  }
  const isError = value['isError'] ?? false;
  if (typeof isError !== 'boolean') {
    throw messageError(`${path}.isError 必须是布尔值`, path);
  }
  const metadata =
    value['metadata'] === undefined
      ? undefined
      : cloneJsonInput(value['metadata'], `${path}.metadata`);
  return Object.freeze({
    type: 'tool_result',
    callId,
    name,
    content,
    isError,
    ...(metadata === undefined ? {} : { metadata }),
  });
}

/** 克隆并冻结 JSON 输入；把 core 的底层错误包装为消息错误。 */
function cloneJsonInput(value: unknown, path: string): JsonValue {
  if (value === undefined) {
    throw messageError(`${path} 必须是 JSON 安全值`, path);
  }
  try {
    return deepFreeze(cloneJsonValue(value, path));
  } catch (error) {
    throw messageError(`${path} 必须是 JSON 安全值`, path, error);
  }
}

/** 校验非空字符串。 */
function requireNonEmptyString(value: unknown, label: string, path: string): string {
  if (typeof value !== 'string' || value.trim() === '') {
    throw messageError(`${label} 必须是非空字符串`, path);
  }
  return value;
}

/** 校验工具名称。 */
function requireToolName(value: unknown, label: string, path: string): string {
  if (!isValidToolName(value)) {
    throw messageError(`${label} 必须匹配 ${TOOL_NAME_PATTERN.source}`, path);
  }
  return value;
}

/** 构造带路径的消息错误。 */
function messageError(
  message: string,
  path: string | undefined,
  cause?: unknown,
): AgentMessageError {
  return new AgentMessageError(message, {
    ...(path === undefined ? {} : { path }),
    ...(cause === undefined ? {} : { cause }),
  });
}
