/**
 * @file 金融 Agent 真实环境验证入口。
 * @description 串联 DeepSeek、Agent 工具循环、金融 provider 与公开数据源；配置由 `@kurumi/config` 加载。
 * @module example-finance-agent/verify
 */

import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

import {
  AgentEventType,
  createAgent,
  createUserMessage,
  extractMessageText,
  type AgentResult,
  type ModelProvider,
} from '@kurumi/agent';
import { LogLevel, createConsoleSink, createLogger, type Logger } from '@kurumi/log';
import { createOpenAIProvider } from '@kurumi/open-ai';

import {
  financeAgentConfigSchema,
  loadFinanceAgentConfig,
  type FinanceAgentConfig,
} from './config.ts';
import { buildFinanceStack } from './finance-stack.ts';

/** 用户任务。 */
const USER_PROMPT = `请调用工具完成以下金融分析，并用中文给出简洁总结：
1. BTC/USDT 最近 60 根 1h K 线：最新价、RSI、MACD 柱状图、布林带 %B。
2. AAPL 最近 60 根日线：最新价、RSI、MACD。
3. SPY 最近到期期权链：ATM 行权价附近 call/put 摘要。
4. EUR/USD 即期汇率与日线指标。
5. 美国国债最新 10Y 收益率。
6. AAPL 最近2个季度基本面：净利率、ROE、Piotroski。
必须使用工具获取数据，不要编造。`;

/** 最终回答输出路径：`examples/finance-agent/report.md`。 */
const REPORT_PATH = fileURLToPath(new URL('../report.md', import.meta.url));

/** 当前日志器；配置加载失败时用于兜底输出错误。 */
let logger: Logger | undefined;

async function main(): Promise<void> {
  const { config, configFile } = await loadFinanceAgentConfig(process.argv[2]);
  const log = createAgentLogger(config.log_level);
  logger = log;

  log.info('配置加载完成', {
    config: financeAgentConfigSchema.redact(config),
    configFile: `examples/finance-agent/${configFile}`,
    envFile: 'examples/finance-agent/.env',
  });

  const stack = buildFinanceStack(config.sec_user_agent);
  log.info('金融 Agent 启动', {
    mode: 'DeepSeek',
    model: config.model,
    toolCount: stack.tools.length,
    tools: stack.toolNames,
  });

  const agent = createAgent({
    provider: createDeepSeekProvider(config),
    model: config.model,
    system: config.system,
    tools: stack.tools,
    toolConcurrency: config.tool_concurrency,
    maxSteps: config.max_steps,
    temperature: config.temperature,
    maxOutputTokens: config.max_output_tokens,
    shouldStream: true,
  });

  let toolCount = 0;
  let failedToolCount = 0;
  let result: AgentResult | undefined;

  for await (const event of agent.stream([createUserMessage(USER_PROMPT)])) {
    switch (event.type) {
      case AgentEventType.ToolStart:
        toolCount += 1;
        log.info('工具调用开始', { tool: event.name, callId: event.callId });
        break;
      case AgentEventType.ToolEnd:
        if (event.result.isError) failedToolCount += 1;
        log.info('工具调用结束', {
          tool: event.name,
          callId: event.callId,
          isError: event.result.isError,
          result: event.result.content,
        });
        break;
      case AgentEventType.StepEnd:
        log.info('Agent 轮次结束', { step: event.step, stopReason: event.stopReason });
        break;
      case AgentEventType.End:
        result = event.result;
        break;
      default:
        break;
    }
  }

  assert.ok(result !== undefined, 'Agent 未返回最终结果');
  assert.ok(toolCount > 0, 'Agent 没有调用任何金融工具');
  assert.equal(failedToolCount, 0, '存在失败的工具调用');
  const finalText =
    (result.finalMessage === undefined ? undefined : extractMessageText(result.finalMessage)) ??
    [...result.messages]
      .reverse()
      .filter((message) => message.role === 'assistant')
      .map((message) => extractMessageText(message))
      .find((text) => text.trim() !== '') ??
    '';
  assert.ok(finalText.trim() !== '', 'Agent 最终回答为空');

  await writeFile(REPORT_PATH, finalText.endsWith('\n') ? finalText : `${finalText}\n`, 'utf8');
  log.info('最终回答已写入报告', { path: 'examples/finance-agent/report.md' });

  log.info('金融 Agent 验证通过', {
    toolCalls: toolCount,
    failedTools: failedToolCount,
    steps: result.steps,
    totalTokens: result.usage.totalTokens,
  });
  log.info('最终回答', { text: finalText });
}

/** 创建结构化日志器。 */
function createAgentLogger(level: LogLevel): Logger {
  return createLogger({
    level,
    sinks: [createConsoleSink({ format: 'pretty' })],
    bindings: { component: 'finance-agent' },
  });
}

/** 创建 DeepSeek（OpenAI 兼容）provider。 */
function createDeepSeekProvider(config: FinanceAgentConfig): ModelProvider {
  return createOpenAIProvider({
    id: 'deepseek',
    apiKey: config.api_key,
    baseUrl: config.base_url,
  });
}

try {
  await main();
} catch (error) {
  logger ??= createAgentLogger(LogLevel.Info);
  logger.error('金融 Agent 验证失败', { error });
  process.exitCode = 1;
} finally {
  await logger?.close();
}
