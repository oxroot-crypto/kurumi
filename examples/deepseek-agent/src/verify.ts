/**
 * @file 真实环境验证示例：DeepSeek + core/config/log/agent/open-ai/anthropic 联动。
 * @description 从 examples/deepseek-agent/.env 读取 KURUMI_API_KEY，校验配置、写日志、执行非流式请求并流式运行带工具的 Agent。
 * @module examples/deepseek-agent/verify
 */

import { fileURLToPath } from 'node:url';

import {
  AgentStopReason,
  createAgent,
  createTool,
  createUserMessage,
  extractMessageReasoning,
  extractMessageText,
  type AgentEvent,
  type AgentResult,
} from '@kurumi/agent';
import { createAnthropicProvider } from '@kurumi/anthropic';
import {
  boolean,
  envSource,
  fileSource,
  loadConfig,
  number,
  object,
  oneOf,
  string,
} from '@kurumi/config';
import { serializeError } from '@kurumi/core';
import { createNodeRuntimeHost } from '@kurumi/core/node';
import { createConsoleSink, createLogger, type Logger } from '@kurumi/log';
import { createOpenAIProvider } from '@kurumi/open-ai';
import {
  number as toolNumber,
  object as toolObject,
  oneOf as toolOneOf,
  string as toolString,
} from '@kurumi/schema';

/** 示例所在目录，用于解析配置文件。 */
const examplesDir = fileURLToPath(new URL('..', import.meta.url));

/** 配置文件路径；可通过命令行参数切换协议，例如 `agent.anthropic.jsonc`。 */
const configPath = process.argv[2] ?? 'agent.jsonc';

/** 验证提示词。 */
const PROMPT = [
  '请按顺序真实调用工具完成验证：',
  '1. 调用 get_current_time 获取 Asia/Shanghai 当前时间；',
  '2. 调用 calculate 计算 21 * 2；',
  '3. 用一句中文总结时间和计算结果。',
  '必须实际调用工具，不要只描述步骤。',
].join('\n');

/** 配置 schema。 */
const schema = object({
  api_key: string({ isNonEmpty: true }).secret(),
  api_format: oneOf(['openai', 'anthropic'] as const).default('openai'),
  base_url: string({ isNonEmpty: true }).default('https://api.deepseek.com'),
  model: string({ isNonEmpty: true }).default('deepseek-chat'),
  temperature: number({ min: 0, max: 2 }).default(0.2),
  max_steps: number({ isInteger: true, min: 1, max: 16 }).default(4),
  max_output_tokens: number({ isInteger: true, min: 1, max: 8_192 }).default(1_024),
  thinking: boolean().default(false),
  thinking_budget_tokens: number({ isInteger: true, min: 1_024, max: 8_192 }).default(2_048),
  reasoning_effort: oneOf(['minimal', 'low', 'medium', 'high'] as const).default('high'),
  log_level: oneOf(['trace', 'debug', 'info', 'warn', 'error'] as const).default('info'),
  system: string({ isNonEmpty: true }).default('你是 Kurumi Agent SDK 的验证助手。'),
});

/** 工具输入 schema：当前时间。 */
const timeInputSchema = toolObject({
  timeZone: toolString({ isNonEmpty: true })
    .describe('IANA 时区名，例如 Asia/Shanghai；缺省 Asia/Shanghai。')
    .default('Asia/Shanghai'),
});

/** 工具输入 schema：计算。 */
const calculateInputSchema = toolObject({
  left: toolNumber(),
  right: toolNumber(),
  operator: toolOneOf(['add', 'subtract', 'multiply', 'divide'] as const),
});

/** 工具输入：计算。 */
interface CalculateInput {
  readonly left: number;
  readonly right: number;
  readonly operator: 'add' | 'subtract' | 'multiply' | 'divide';
}

let logger: Logger | undefined;

try {
  const host = createNodeRuntimeHost({ cwd: examplesDir, env: process.env });
  const config = await loadConfig({
    schema,
    host,
    sources: [fileSource({ path: configPath, isOptional: true }), envSource({ prefix: 'KURUMI_' })],
    signal: AbortSignal.timeout(10_000),
  });

  logger = createLogger({
    level: config.log_level,
    sinks: [createConsoleSink({ format: 'pretty' })],
    bindings: { example: 'deepseek-agent' },
  });
  logger.info('配置加载完成', {
    config: schema.redact(config),
    configFile: `examples/deepseek-agent/${configPath}`,
    envFile: 'examples/deepseek-agent/.env',
    thinking: config.thinking,
    reasoningEffort: config.reasoning_effort,
  });

  const provider =
    config.api_format === 'anthropic'
      ? createAnthropicProvider({
          id: 'deepseek',
          apiKey: config.api_key,
          baseUrl: config.base_url,
        })
      : createOpenAIProvider({
          id: 'deepseek',
          apiKey: config.api_key,
          baseUrl: config.base_url,
        });

  const directResponse = await provider.generate(
    {
      model: config.model,
      messages: [createUserMessage('请只回复：OK')],
      tools: [],
      temperature: 0,
      maxOutputTokens: 256,
    },
    { signal: AbortSignal.timeout(30_000) },
  );
  const directText = extractMessageText(directResponse.message);
  if (directText === '') throw new Error('非流式生成没有文本回答');
  logger.info('非流式生成验证通过', {
    stopReason: directResponse.stopReason,
    usage: directResponse.usage,
    text: directText,
  });

  const agent = createAgent({
    provider,
    model: config.model,
    system: config.system,
    tools: [
      createTool({
        name: 'get_current_time',
        description: '返回指定 IANA 时区的当前时间；缺省 Asia/Shanghai。',
        inputSchema: timeInputSchema,
        execute: ({ timeZone }) => {
          const now = new Date();
          const formatted = new Intl.DateTimeFormat('zh-CN', {
            timeZone,
            dateStyle: 'full',
            timeStyle: 'long',
          }).format(now);
          return {
            content: `${formatted}（${timeZone}）`,
            metadata: { timeZone, iso: now.toISOString() },
          };
        },
      }),
      createTool({
        name: 'calculate',
        description: '对两个数字执行四则运算并返回结果。',
        inputSchema: calculateInputSchema,
        execute: ({ left, right, operator }) => {
          const result = calculate(left, right, operator);
          return {
            content: `${String(left)} ${operatorSymbol(operator)} ${String(right)} = ${String(result)}`,
            metadata: { left, right, operator, result },
          };
        },
      }),
    ],
    maxSteps: config.max_steps,
    toolConcurrency: 2,
    toolTimeoutMs: 10_000,
    maxToolResultLength: 4_000,
    temperature: config.temperature,
    maxOutputTokens: config.max_output_tokens,
    thinking: config.thinking
      ? {
          isEnabled: true,
          budgetTokens: config.thinking_budget_tokens,
          effort: config.reasoning_effort,
        }
      : { isEnabled: false },
    maxModelRetries: 1,
    retryDelayMs: 500,
    approveToolCall: (call) => {
      logger?.info('工具审批通过', { tool: call.name, callId: call.callId });
      return true;
    },
  });

  const runtime = await runAgentStream(
    agent.stream([createUserMessage(PROMPT)], {
      signal: AbortSignal.timeout(90_000),
    }),
  );

  assertVerification(runtime.result, runtime.toolNames, runtime.hasToolError);

  if (
    config.thinking &&
    runtime.reasoningDeltaCount === 0 &&
    generatedReasoningOf(runtime.result) === ''
  ) {
    logger.warn('已启用 thinking，但模型未返回推理内容；请确认模型与端点支持 thinking');
  }
  logger.info('✅ 验证通过', {
    model: config.model,
    stopReason: runtime.result.stopReason,
    steps: runtime.result.steps,
    usage: runtime.result.usage,
    retryCount: runtime.result.retryCount,
    toolCalls: [...runtime.toolNames],
    deltaCount: runtime.deltaCount,
    reasoningDeltaCount: runtime.reasoningDeltaCount,
    reasoning: generatedReasoningOf(runtime.result),
    finalText: finalTextOf(runtime.result),
  });
} catch (error) {
  if (logger === undefined) {
    console.error(JSON.stringify(serializeError(error), null, 2));
  } else {
    logger.error('❌ 验证失败', { error });
  }
  process.exitCode = 1;
} finally {
  if (logger !== undefined) {
    try {
      await logger.close();
    } catch (error) {
      console.error('关闭 logger 失败', serializeError(error));
      process.exitCode = 1;
    }
  }
}

/** 运行事件流并收集验证信息。 */
async function runAgentStream(stream: AsyncGenerator<AgentEvent, AgentResult, void>): Promise<{
  readonly result: AgentResult;
  readonly toolNames: ReadonlySet<string>;
  readonly hasToolError: boolean;
  readonly deltaCount: number;
  readonly reasoningDeltaCount: number;
}> {
  const toolNames = new Set<string>();
  let hasToolError = false;
  let deltaCount = 0;
  let reasoningDeltaCount = 0;
  let next = await stream.next();
  while (next.done !== true) {
    const event = next.value;
    switch (event.type) {
      case 'agent_start':
        logger?.info('Agent 启动', {
          providerId: event.providerId,
          model: event.model,
          messages: event.messages.length,
        });
        break;
      case 'step_start':
        logger?.debug('模型轮次开始', { step: event.step });
        break;
      case 'model_delta':
        deltaCount += 1;
        if (event.delta.type === 'reasoning_delta') reasoningDeltaCount += 1;
        logger?.trace('模型增量', { step: event.step, delta: event.delta });
        break;
      case 'model_retry':
        logger?.warn('模型调用重试', {
          step: event.step,
          attempt: event.attempt,
          delayMs: event.delayMs,
          error: event.error,
        });
        break;
      case 'assistant_message':
        logger?.info('assistant 消息', {
          step: event.step,
          stopReason: event.stopReason,
          usage: event.usage,
          text: extractMessageText(event.message),
          reasoning: extractMessageReasoning(event.message),
        });
        break;
      case 'tool_start':
        logger?.info('工具开始', {
          step: event.step,
          callId: event.callId,
          tool: event.name,
          input: event.input,
        });
        break;
      case 'tool_end':
        toolNames.add(event.name);
        hasToolError ||= event.result.isError;
        logger?.info('工具结束', {
          step: event.step,
          callId: event.callId,
          tool: event.name,
          isError: event.result.isError,
          content: event.result.content,
          metadata: event.result.metadata,
        });
        break;
      case 'step_end':
        logger?.debug('模型轮次结束', { step: event.step, stopReason: event.stopReason });
        break;
      case 'agent_end':
        logger?.info('Agent 结束', {
          stopReason: event.result.stopReason,
          steps: event.result.steps,
          usage: event.result.usage,
          retryCount: event.result.retryCount,
        });
        break;
      default:
        logUnknownEvent(event);
    }
    next = await stream.next();
  }
  return { result: next.value, toolNames, hasToolError, deltaCount, reasoningDeltaCount };
}

/** 校验真实运行结果，确保三个包的能力都被执行到。 */
function assertVerification(
  result: AgentResult,
  toolNames: ReadonlySet<string>,
  hasToolError: boolean,
): void {
  if (hasToolError) {
    throw new Error('工具执行返回了错误结果，验证失败');
  }
  if (result.stopReason !== AgentStopReason.Completed) {
    throw new Error(`Agent 未正常完成: ${result.stopReason}`);
  }
  if (!toolNames.has('get_current_time')) {
    throw new Error('模型没有调用 get_current_time，工具链路未验证');
  }
  if (!toolNames.has('calculate')) {
    throw new Error('模型没有调用 calculate，工具链路未验证');
  }
  if (result.generatedMessages.length === 0) {
    throw new Error('没有生成任何消息');
  }
  if (finalTextOf(result) === '') {
    throw new Error('缺少最终文本回答');
  }
}

/** 提取最终回答。 */
function finalTextOf(result: AgentResult): string {
  return result.finalMessage === undefined ? '' : extractMessageText(result.finalMessage);
}

/** 汇总本次运行所有 assistant 消息的推理文本。 */
function generatedReasoningOf(result: AgentResult): string {
  return result.generatedMessages.map((message) => extractMessageReasoning(message)).join('');
}

/** 四则运算。 */
function calculate(left: number, right: number, operator: CalculateInput['operator']): number {
  switch (operator) {
    case 'add':
      return left + right;
    case 'subtract':
      return left - right;
    case 'multiply':
      return left * right;
    case 'divide':
      if (right === 0) throw new Error('除数不能为 0');
      return left / right;
  }
}

/** 运算符展示。 */
function operatorSymbol(operator: CalculateInput['operator']): string {
  switch (operator) {
    case 'add':
      return '+';
    case 'subtract':
      return '-';
    case 'multiply':
      return '×';
    case 'divide':
      return '÷';
  }
}

/** 理论上不会出现的未知事件；显式记录便于 SDK 演进时发现遗漏。 */
function logUnknownEvent(event: never): void {
  logger?.warn('未知 Agent 事件', { event });
}
