/**
 * @file finance-agent 配置 schema 与加载入口。
 * @description 使用 `@kurumi/config` 合并 `agent.jsonc` 与 `KURUMI_*` 环境变量；密钥只以脱敏形式进入日志。
 * @module example-finance-agent/config
 */

import { fileURLToPath } from 'node:url';

import {
  envSource,
  fileSource,
  loadConfig,
  number,
  object,
  oneOf,
  string,
  type Infer,
} from '@kurumi/config';
import { createNodeRuntimeHost } from '@kurumi/core/node';

/** example 根目录；作为配置宿主的 cwd，避免脚本依赖启动目录。 */
export const EXAMPLE_DIR = fileURLToPath(new URL('..', import.meta.url));

/** 金融 Agent 默认 system prompt。 */
export const DEFAULT_SYSTEM_PROMPT =
  '你是严谨的金融分析 Agent。所有行情、指标、基本面与收益率数据必须来自工具；不要编造数值。工具结果是唯一事实来源。';

/** 配置文件默认名称；可通过第一个命令行参数覆盖。 */
export const DEFAULT_CONFIG_FILE = 'agent.jsonc';

/**
 * 配置 schema。
 *
 * @remarks
 * `api_key` 标记为 secret，`schema.redact(config)` 会输出 `[REDACTED]`；
 * `KURUMI_*` 环境变量由 `envSource` 映射为同名字段并覆盖配置文件。
 */
export const financeAgentConfigSchema = object({
  api_key: string({ isNonEmpty: true })
    .secret()
    .describe('DeepSeek/OpenAI 兼容 API Key；可由 KURUMI_API_KEY 或 DEEPSEEK_API_KEY 提供'),
  base_url: string({ isNonEmpty: true })
    .default('https://api.deepseek.com')
    .describe('模型 API 根地址'),
  model: string({ isNonEmpty: true }).default('deepseek-chat').describe('模型名称'),
  sec_user_agent: string({ isNonEmpty: true }).describe('SEC 要求的 User-Agent，必须包含联系方式'),
  temperature: number({ min: 0, max: 2 }).default(0.1).describe('采样温度'),
  max_steps: number({ isInteger: true, min: 1, max: 32 }).default(10).describe('Agent 最大轮次'),
  max_output_tokens: number({ isInteger: true, min: 256, max: 32_768 })
    .default(2_048)
    .describe('单轮最大输出 Token'),
  tool_concurrency: number({ isInteger: true, min: 1, max: 8 }).default(2).describe('工具并发上限'),
  log_level: oneOf(['trace', 'debug', 'info', 'warn', 'error', 'fatal'] as const)
    .default('info')
    .describe('日志级别'),
  system: string({ isNonEmpty: true })
    .default(DEFAULT_SYSTEM_PROMPT)
    .describe('Agent system prompt'),
});

/** 已校验的配置类型。 */
export type FinanceAgentConfig = Infer<typeof financeAgentConfigSchema>;

/** 配置加载结果。 */
export interface LoadedFinanceAgentConfig {
  /** 已校验并冻结的配置。 */
  readonly config: FinanceAgentConfig;
  /** 实际使用的配置文件路径（相对 example 根目录）。 */
  readonly configFile: string;
}

/**
 * 加载 finance-agent 配置。
 *
 * @remarks
 * 来源优先级：`agent.jsonc` < `KURUMI_*` 环境变量。
 * 环境变量通常由 `node --env-file=.env src/verify.ts` 注入。
 *
 * @param configFile - 配置文件名，相对 example 根目录；缺省 `agent.jsonc`
 * @returns 配置与配置文件名
 * @throws {ConfigSourceError} 配置文件读取/解析失败
 * @throws {ConfigValidationError} 缺少必填项或字段非法
 */
export async function loadFinanceAgentConfig(
  configFile: string = DEFAULT_CONFIG_FILE,
): Promise<LoadedFinanceAgentConfig> {
  const host = createNodeRuntimeHost({ cwd: EXAMPLE_DIR, env: process.env });
  const config = await loadConfig({
    schema: financeAgentConfigSchema,
    host,
    sources: [fileSource({ path: configFile, isOptional: true }), envSource({ prefix: 'KURUMI_' })],
    signal: AbortSignal.timeout(10_000),
  });

  return Object.freeze({ config, configFile });
}
