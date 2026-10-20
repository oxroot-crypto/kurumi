/**
 * @file 工具定义、结果契约与工具注册表。
 * @description 工具是 Agent 与外部世界交互的唯一入口；执行失败会被归一化为模型可见的结果。
 * @module agent/tool
 */

import { cloneJsonValue, deepFreeze, isPlainObject, type JsonValue } from '@kurumi/core';
import { Schema } from '@kurumi/schema';

import { AgentDefinitionError, AgentToolError } from './errors.js';
import { isValidToolName, type AgentMessage } from './message.js';

/** 暴露给模型的工具描述。 */
export interface ToolDescriptor {
  /** 工具名称，必须匹配 `^[A-Za-z][A-Za-z0-9_-]{0,63}$`。 */
  readonly name: string;
  /** 面向模型的用途说明。 */
  readonly description: string;
  /** JSON Schema 形态的输入描述。 */
  readonly inputSchema: JsonValue;
}

/** 工具执行结果。 */
export interface ToolResult {
  /** 面向模型的文本结果。 */
  readonly content: string;
  /** 是否以失败结束；失败结果仍会回传给模型。 */
  readonly isError: boolean;
  /** 可选的 JSON 安全元数据，供上层程序消费。 */
  readonly metadata?: JsonValue;
}

/** 工具实现可直接返回的结果输入；`isError` 缺省为 false。 */
export interface ToolResultInput {
  /** 面向模型的文本结果。 */
  readonly content: string;
  /** 是否以失败结束。 */
  readonly isError?: boolean;
  /** 可选的 JSON 安全元数据。 */
  readonly metadata?: JsonValue;
}

/** 工具实现可以返回字符串或结构化结果。 */
export type ToolOutput = string | ToolResultInput;

/** 工具执行上下文。 */
export interface ToolContext {
  /** 模型提供的调用 ID。 */
  readonly callId: string;
  /** 工具名称。 */
  readonly name: string;
  /** 取消信号；Agent 中止或超时时会触发。 */
  readonly signal: AbortSignal;
  /** 调用发生时的完整对话快照。 */
  readonly messages: readonly AgentMessage[];
}

/**
 * 工具输入 schema：既可以是 `@kurumi/schema` 的强类型 schema，也可以是原始 JSON Schema。
 *
 * @remarks
 * - 传入 `Schema<TInput>` 时，`createTool` 会自动导出 JSON Schema，并默认用 `parseOrThrow` 校验输入；
 * - 传入原始 JSON Schema 时，可通过 `parseInput` 自行完成校验/转换。
 */
export type ToolInputSchema<TInput = unknown> = Schema<TInput> | JsonValue;

/** 工具定义。 */
export interface ToolDefinition<TInput = unknown> {
  /** 工具名称。 */
  readonly name: string;
  /** 面向模型的用途说明，非空字符串。 */
  readonly description: string;
  /** 强类型 schema 或 JSON Schema 形态的输入描述。 */
  readonly inputSchema: ToolInputSchema<TInput>;
  /** 可选的输入校验/转换函数；抛错时结果会被标记为错误并回传模型。 */
  parseInput?(input: JsonValue): TInput;
  /** 实际执行逻辑；抛错时结果会被标记为错误并回传模型。 */
  execute(input: TInput, context: ToolContext): ToolOutput | Promise<ToolOutput>;
  /** 单次调用超时（毫秒），缺省使用 Agent 级配置。 */
  readonly timeoutMs?: number;
}

/** 已完成校验与 JSON Schema 归一化的工具定义。 */
export interface NormalizedToolDefinition<TInput = unknown> extends Omit<
  ToolDefinition<TInput>,
  'inputSchema'
> {
  /** 已冻结的 JSON Schema。 */
  readonly inputSchema: JsonValue;
}

/** 工具注册表。 */
export interface ToolRegistry {
  /** 工具数量。 */
  readonly size: number;
  /** 冻结的工具描述数组，顺序与创建时一致。 */
  readonly descriptors: readonly ToolDescriptor[];
  /**
   * 判断工具是否存在。
   *
   * @param name - 工具名称
   * @returns 存在时返回 true
   */
  has(name: string): boolean;
  /**
   * 按名称查找工具。
   *
   * @param name - 工具名称
   * @returns 找到时返回工具定义，否则返回 undefined
   */
  get(name: string): NormalizedToolDefinition<unknown> | undefined;
}

/**
 * 校验并冻结工具定义。
 *
 * @param definition - 工具定义
 * @returns 冻结的工具定义；输入 schema 被独立克隆
 * @throws {AgentDefinitionError} 名称、描述、schema、函数或超时非法
 *
 * @example
 * ```ts
 * import { object, string } from '@kurumi/schema';
 *
 * const search = createTool({
 *   name: 'search',
 *   description: '搜索文档',
 *   inputSchema: object({ query: string({ isNonEmpty: true }) }),
 *   execute: async ({ query }, { signal }) => `结果: ${query}`,
 * });
 * ```
 */
export function createTool<TInput = unknown>(
  definition: ToolDefinition<TInput>,
): NormalizedToolDefinition<TInput> {
  if (typeof definition !== 'object' || definition === null) {
    throw new AgentDefinitionError('工具定义必须是对象');
  }
  assertToolName(definition.name);
  if (typeof definition.description !== 'string' || definition.description.trim() === '') {
    throw new AgentDefinitionError(`工具 "${definition.name}" 的 description 不能为空`);
  }
  if (typeof definition.execute !== 'function') {
    throw new AgentDefinitionError(`工具 "${definition.name}" 的 execute 必须是函数`);
  }
  if (definition.parseInput !== undefined && typeof definition.parseInput !== 'function') {
    throw new AgentDefinitionError(`工具 "${definition.name}" 的 parseInput 必须是函数`);
  }
  if (
    definition.timeoutMs !== undefined &&
    (!Number.isSafeInteger(definition.timeoutMs) || definition.timeoutMs <= 0)
  ) {
    throw new AgentDefinitionError(`工具 "${definition.name}" 的 timeoutMs 必须是正整数`);
  }

  const schema = definition.inputSchema instanceof Schema ? definition.inputSchema : undefined;
  const inputSchema =
    schema === undefined
      ? cloneToolSchema(definition.name, definition.inputSchema)
      : schemaToToolInputSchema(definition.name, schema);
  const parseInput =
    definition.parseInput ??
    (schema === undefined ? undefined : (input: JsonValue): TInput => schema.parseOrThrow(input));

  return Object.freeze({
    name: definition.name,
    description: definition.description,
    inputSchema,
    ...(parseInput === undefined ? {} : { parseInput }),
    execute: definition.execute,
    ...(definition.timeoutMs === undefined ? {} : { timeoutMs: definition.timeoutMs }),
  });
}

/**
 * 创建工具注册表并拒绝重名。
 *
 * @param tools - 工具定义数组；缺省为空
 * @returns 冻结的注册表，描述顺序与输入一致
 * @throws {AgentDefinitionError} 工具数组非法、定义非法或名称重复
 *
 * @example
 * ```ts
 * const registry = createToolRegistry([search, readFile]);
 * registry.descriptors.map((tool) => tool.name); // ['search', 'readFile']
 * ```
 */
export function createToolRegistry(tools: readonly ToolDefinition[] = []): ToolRegistry {
  if (!Array.isArray(tools)) {
    throw new AgentDefinitionError('tools 必须是工具定义数组');
  }
  const byName = new Map<string, NormalizedToolDefinition<unknown>>();
  for (const tool of tools) {
    const normalized = createTool(tool);
    if (byName.has(normalized.name)) {
      throw new AgentDefinitionError(`工具名称重复: "${normalized.name}"`);
    }
    byName.set(normalized.name, normalized);
  }

  const descriptors = Object.freeze(
    [...byName.values()].map((tool) =>
      Object.freeze({
        name: tool.name,
        description: tool.description,
        inputSchema: tool.inputSchema,
      }),
    ),
  );

  return Object.freeze({
    size: byName.size,
    descriptors,
    has: (name: string): boolean => byName.has(name),
    get: (name: string): NormalizedToolDefinition<unknown> | undefined => byName.get(name),
  });
}

/**
 * 校验并归一化工具返回值。
 *
 * @param value - 工具实现返回的原始值
 * @param toolName - 工具名称，用于错误上下文
 * @returns 冻结的结果对象
 * @throws {AgentToolError} 返回值不是字符串或合法的 ToolResultInput
 */
export function normalizeToolResult(value: unknown, toolName: string): ToolResult {
  if (typeof value === 'string') {
    return Object.freeze({ content: value, isError: false });
  }
  if (!isPlainObject(value)) {
    throw new AgentToolError(toolName, '返回了非法结果：必须是字符串或普通对象');
  }
  const content = value['content'];
  if (typeof content !== 'string') {
    throw new AgentToolError(toolName, '返回了非法结果：content 必须是字符串');
  }
  const isError = value['isError'] ?? false;
  if (typeof isError !== 'boolean') {
    throw new AgentToolError(toolName, '返回了非法结果：isError 必须是布尔值');
  }
  const metadata = normalizeToolMetadata(toolName, value['metadata']);
  return Object.freeze({
    content,
    isError,
    ...(metadata === undefined ? {} : { metadata }),
  });
}

/** 克隆并冻结工具输入 schema。 */
function cloneToolSchema(toolName: string, value: unknown): JsonValue {
  if (!isPlainObject(value)) {
    throw new AgentDefinitionError(`工具 "${toolName}" 的 inputSchema 必须是 JSON Schema 对象`);
  }
  try {
    return deepFreeze(cloneJsonValue(value, '$.tool.inputSchema'));
  } catch (error) {
    throw new AgentDefinitionError(`工具 "${toolName}" 的 inputSchema 必须是 JSON 安全值`, error);
  }
}

/** 把强类型 schema 导出为冻结的 JSON Schema。 */
function schemaToToolInputSchema(toolName: string, schema: Schema<unknown>): JsonValue {
  try {
    return deepFreeze(schema.toJsonSchema());
  } catch (error) {
    throw new AgentDefinitionError(
      `工具 "${toolName}" 的 inputSchema 无法转换为 JSON Schema`,
      error,
    );
  }
}

/** 校验工具名称。 */
function assertToolName(value: unknown): asserts value is string {
  if (!isValidToolName(value)) {
    throw new AgentDefinitionError('工具名称必须匹配 ^[A-Za-z][A-Za-z0-9_-]{0,63}$');
  }
}

/** 克隆并冻结工具结果元数据。 */
function normalizeToolMetadata(toolName: string, value: unknown): JsonValue | undefined {
  if (value === undefined) return undefined;
  try {
    return deepFreeze(cloneJsonValue(value, '$.tool.metadata'));
  } catch (error) {
    throw new AgentToolError(toolName, '返回了非法结果：metadata 必须是 JSON 安全值', error);
  }
}
