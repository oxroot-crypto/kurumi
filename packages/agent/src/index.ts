/**
 * @file `@kurumi/agent` 主入口。
 * @description 跨 runtime Agent SDK 基础设施；桶导出，不承载业务逻辑。
 * @module agent
 */

export {
  DEFAULT_MAX_MODEL_RETRIES,
  DEFAULT_MAX_STEPS,
  DEFAULT_MAX_TOOL_RESULT_LENGTH,
  DEFAULT_RETRY_DELAY_MS,
  DEFAULT_TOOL_CONCURRENCY,
  type Agent,
  type AgentConfig,
  type AgentRunOptions,
  type ToolApprovalCallback,
  type ToolApprovalContext,
} from './agent-types.js';
export { createAgent, runAgent, streamAgent } from './agent.js';
export {
  AgentDefinitionError,
  AgentError,
  AgentMessageError,
  AgentModelError,
  AgentToolError,
} from './errors.js';
export {
  AgentEventType,
  type AgentAssistantMessageEvent,
  type AgentEndEvent,
  type AgentEvent,
  type AgentModelDeltaEvent,
  type AgentModelRetryEvent,
  type AgentStartEvent,
  type AgentStepEndEvent,
  type AgentStepStartEvent,
  type AgentToolEndEvent,
  type AgentToolStartEvent,
} from './events.js';
export {
  AgentMessageRole,
  createAssistantMessage,
  createMessage,
  createSystemMessage,
  createToolMessage,
  createUserMessage,
  extractMessageReasoning,
  extractMessageText,
  getToolCalls,
  isValidToolName,
  normalizeAgentMessage,
  type AgentContentPart,
  type AgentMessage,
  type AgentMessageContent,
  type CreateToolMessageInput,
  type ImageContentPart,
  type ReasoningContentPart,
  type TextContentPart,
  type ToolCallContentPart,
  type ToolResultContentPart,
} from './message.js';
export {
  ModelStopReason,
  ThinkingEffort,
  isModelStopReason,
  isThinkingEffort,
  normalizeThinkingConfig,
  normalizeToolChoice,
  type ModelCallOptions,
  type ModelDelta,
  type ModelProvider,
  type ModelReasoningDelta,
  type ModelRequest,
  type ModelResponse,
  type ModelResponseEvent,
  type ModelStreamEvent,
  type ModelTextDelta,
  type ModelToolCallDelta,
  type ResolvedThinkingConfig,
  type ThinkingConfig,
  type ThinkingOption,
  type ToolChoice,
} from './model.js';
export { AgentStopReason, type AgentResult } from './result.js';
export {
  createAgentSession,
  type AgentSession,
  type CreateAgentSessionOptions,
} from './session.js';
export {
  createTool,
  createToolRegistry,
  type ToolContext,
  type NormalizedToolDefinition,
  type ToolDefinition,
  type ToolDescriptor,
  type ToolInputSchema,
  type ToolOutput,
  type ToolRegistry,
  type ToolResult,
  type ToolResultInput,
} from './tool.js';
export {
  EMPTY_TOKEN_USAGE,
  addTokenUsage,
  createTokenUsage,
  normalizeTokenUsage,
  type TokenUsage,
  type TokenUsageOptions,
} from './usage.js';
