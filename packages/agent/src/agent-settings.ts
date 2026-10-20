/**
 * @file Agent 配置解析与单次运行准备。
 * @description 只做定义期校验与不可变快照构造，不执行模型或工具。
 * @module agent/agent-settings
 */

import { isAbortSignal } from '@kurumi/core';

import {
  DEFAULT_MAX_MODEL_RETRIES,
  DEFAULT_MAX_STEPS,
  DEFAULT_MAX_TOOL_RESULT_LENGTH,
  DEFAULT_RETRY_DELAY_MS,
  DEFAULT_TOOL_CONCURRENCY,
  type AgentConfig,
  type AgentRunOptions,
  type AgentSettings,
  type PreparedRun,
  type ToolApprovalCallback,
} from './agent-types.js';
import { AgentDefinitionError } from './errors.js';
import {
  AgentMessageRole,
  createSystemMessage,
  normalizeAgentMessage,
  type AgentMessage,
} from './message.js';
import {
  normalizeProviderOptions,
  normalizeThinkingConfig,
  normalizeToolChoice,
  type ModelProvider,
  type ResolvedThinkingConfig,
  type ThinkingOption,
  type ToolChoice,
} from './model.js';
import { createToolRegistry, type ToolRegistry } from './tool.js';

/** 默认重试判定：只要未取消就重试。 */
const DEFAULT_RETRY_PREDICATE = (): boolean => true;

/**
 * 校验并解析 Agent 配置。
 *
 * @param config - 用户提供的 Agent 配置
 * @returns 不可变运行设置
 * @throws {AgentDefinitionError} 配置、工具或运行选项非法
 */
export function resolveAgentSettings(config: AgentConfig): AgentSettings {
  if (typeof config !== 'object' || config === null) {
    throw new AgentDefinitionError('Agent 配置必须是对象');
  }
  const provider = resolveProvider(config.provider);
  const providerId = requireNonEmptyString(provider.id, 'provider.id');
  const model = requireNonEmptyString(config.model, 'model');
  const registry = createToolRegistry(config.tools ?? []);
  const system =
    config.system === undefined ? undefined : requireNonEmptyString(config.system, 'system');
  const approveToolCall = resolveApprovalCallback(config.approveToolCall);
  const shouldRetryModelError = config.shouldRetryModelError ?? DEFAULT_RETRY_PREDICATE;
  if (typeof shouldRetryModelError !== 'function') {
    throw new AgentDefinitionError('shouldRetryModelError 必须是函数');
  }

  return {
    provider,
    providerId,
    model,
    system,
    registry,
    maxSteps: resolvePositiveInteger(config.maxSteps, DEFAULT_MAX_STEPS, 'maxSteps'),
    toolConcurrency: resolvePositiveInteger(
      config.toolConcurrency,
      DEFAULT_TOOL_CONCURRENCY,
      'toolConcurrency',
    ),
    toolTimeoutMs: resolveOptionalPositiveInteger(config.toolTimeoutMs, 'toolTimeoutMs'),
    maxToolResultLength: resolvePositiveInteger(
      config.maxToolResultLength,
      DEFAULT_MAX_TOOL_RESULT_LENGTH,
      'maxToolResultLength',
    ),
    temperature: resolveOptionalFiniteNumber(config.temperature, 'temperature'),
    maxOutputTokens: resolveOptionalPositiveInteger(config.maxOutputTokens, 'maxOutputTokens'),
    thinking: resolveThinking(config.thinking, 'thinking'),
    providerOptions:
      config.providerOptions === undefined
        ? undefined
        : normalizeProviderOptions(config.providerOptions),
    shouldStream: resolveBoolean(config.shouldStream, true, 'shouldStream'),
    maxModelRetries: resolveNonNegativeInteger(
      config.maxModelRetries,
      DEFAULT_MAX_MODEL_RETRIES,
      'maxModelRetries',
    ),
    retryDelayMs: resolveNonNegativeInteger(
      config.retryDelayMs,
      DEFAULT_RETRY_DELAY_MS,
      'retryDelayMs',
    ),
    shouldRetryModelError,
    approveToolCall,
    toolChoice: resolveToolChoice(config.toolChoice, registry, 'toolChoice'),
  };
}

/**
 * 校验并准备单次运行。
 *
 * @param settings - 已解析的 Agent 设置
 * @param messages - 调用方消息
 * @param options - 单次运行覆盖项
 * @returns 冻结的运行快照
 * @throws {AgentDefinitionError} 消息或运行选项非法
 */
export function prepareRun(
  settings: AgentSettings,
  messages: readonly AgentMessage[],
  options: AgentRunOptions | undefined,
): PreparedRun {
  if (options !== undefined && (typeof options !== 'object' || options === null)) {
    throw new AgentDefinitionError('运行选项必须是对象');
  }
  if (!Array.isArray(messages)) {
    throw new AgentDefinitionError('messages 必须是消息数组');
  }
  const normalized = messages.map((message, index) =>
    normalizeAgentMessage(message, `$.messages[${index}]`),
  );
  // 首条 system 消息视为本次会话的系统提示，避免把 Agent 配置的 system 重复注入。
  const hasSystem = normalized[0]?.role === AgentMessageRole.System;
  const withSystem =
    settings.system === undefined || hasSystem
      ? normalized
      : [createSystemMessage(settings.system), ...normalized];
  if (withSystem.length === 0) {
    throw new AgentDefinitionError('messages 不能为空；请至少传入一条消息或配置 system');
  }

  return Object.freeze({
    messages: Object.freeze(withSystem),
    maxSteps:
      options?.maxSteps === undefined
        ? settings.maxSteps
        : resolvePositiveInteger(options.maxSteps, settings.maxSteps, 'maxSteps'),
    toolChoice:
      options?.toolChoice === undefined
        ? settings.toolChoice
        : resolveToolChoice(options.toolChoice, settings.registry, 'toolChoice'),
    thinking:
      options?.thinking === undefined
        ? settings.thinking
        : resolveThinking(options.thinking, 'thinking'),
    shouldStream: resolveBoolean(options?.shouldStream, settings.shouldStream, 'shouldStream'),
    signal: resolveSignal(options?.signal),
  });
}

/** 校验提供方结构。 */
function resolveProvider(provider: ModelProvider): ModelProvider {
  if (typeof provider !== 'object' || provider === null) {
    throw new AgentDefinitionError('provider 必须是对象');
  }
  if (typeof provider.generate !== 'function') {
    throw new AgentDefinitionError('provider.generate 必须是函数');
  }
  if (provider.stream !== undefined && typeof provider.stream !== 'function') {
    throw new AgentDefinitionError('provider.stream 必须是函数');
  }
  return provider;
}

/** 校验审批回调。 */
function resolveApprovalCallback(value: unknown): ToolApprovalCallback | undefined {
  if (value === undefined) return undefined;
  if (!isApprovalCallback(value)) {
    throw new AgentDefinitionError('approveToolCall 必须是函数');
  }
  return value;
}

/** 判断值是否为审批回调。 */
function isApprovalCallback(value: unknown): value is ToolApprovalCallback {
  return typeof value === 'function';
}

/** 校验工具选择策略与工具存在性。 */
function resolveToolChoice(
  value: ToolChoice | undefined,
  registry: ToolRegistry,
  label: string,
): ToolChoice | undefined {
  if (value === undefined) return undefined;
  const choice = normalizeToolChoice(value, `$.${label}`);
  if (typeof choice === 'object' && !registry.has(choice.name)) {
    throw new AgentDefinitionError(`toolChoice 引用了未注册的工具 "${choice.name}"`);
  }
  if (choice === 'required' && registry.size === 0) {
    throw new AgentDefinitionError('toolChoice=required 需要至少一个工具');
  }
  return choice;
}

/** 校验并归一化思考配置。 */
function resolveThinking(
  value: ThinkingOption | undefined,
  name: string,
): ResolvedThinkingConfig | undefined {
  if (value === undefined) return undefined;
  return normalizeThinkingConfig(value, `$.${name}`);
}

/** 校验正整数，缺省时使用默认值。 */
function resolvePositiveInteger(value: unknown, defaultValue: number, name: string): number {
  if (value === undefined) return defaultValue;
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value <= 0) {
    throw new AgentDefinitionError(`${name} 必须是正整数`);
  }
  return value;
}

/** 校验可选正整数。 */
function resolveOptionalPositiveInteger(value: unknown, name: string): number | undefined {
  if (value === undefined) return undefined;
  return resolvePositiveInteger(value, 1, name);
}

/** 校验非负整数。 */
function resolveNonNegativeInteger(value: unknown, defaultValue: number, name: string): number {
  if (value === undefined) return defaultValue;
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0) {
    throw new AgentDefinitionError(`${name} 必须是非负安全整数`);
  }
  return value;
}

/** 校验可选有限数值。 */
function resolveOptionalFiniteNumber(value: unknown, name: string): number | undefined {
  if (value === undefined) return undefined;
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    throw new AgentDefinitionError(`${name} 必须是有限数值`);
  }
  return value;
}

/** 校验布尔选项。 */
function resolveBoolean(value: unknown, defaultValue: boolean, name: string): boolean {
  if (value === undefined) return defaultValue;
  if (typeof value !== 'boolean') {
    throw new AgentDefinitionError(`${name} 必须是布尔值`);
  }
  return value;
}

/** 校验非空字符串。 */
function requireNonEmptyString(value: unknown, name: string): string {
  if (typeof value !== 'string' || value.trim() === '') {
    throw new AgentDefinitionError(`${name} 必须是非空字符串`);
  }
  return value;
}

/** 校验 AbortSignal 形态。 */
function resolveSignal(value: unknown): AbortSignal | undefined {
  if (value === undefined) return undefined;
  if (!isAbortSignal(value)) {
    throw new AgentDefinitionError('signal 必须是 AbortSignal');
  }
  return value;
}
