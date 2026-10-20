# deepseek-agent

真实环境验证包：用 `examples/deepseek-agent/.env` 中的 `DEEPSEEK_API_KEY`（或 `KURUMI_API_KEY`）调用 DeepSeek，并串联验证七个基础包：

| 包                     | 验证点                                                                |
| ---------------------- | --------------------------------------------------------------------- |
| `@kurumi/core`         | `createNodeRuntimeHost`、`serializeError`、JSON 安全边界              |
| `@kurumi/schema`       | 工具输入 schema、JSON Schema 导出、默认值与自动输入校验               |
| `@kurumi/config`       | JSONC 文件来源、环境变量来源、schema 校验/强制转换、secret `redact`   |
| `@kurumi/log`          | 级别、结构化字段、console sink、错误序列化、`close()` 生命周期        |
| `@kurumi/agent`        | 消息/工具/模型契约、流式增量、工具审批与并发、事件流、usage、会话结果 |
| `@kurumi/open-ai`      | 通用 OpenAI 协议适配：Chat Completions、工具调用、SSE 流式、usage     |
| `@kurumi/anthropic`    | Anthropic Messages 协议适配：工具调用、thinking、SSE 流式、usage      |
| DeepSeek API           | 真实 `chat/completions` 流式调用、工具调用与最终回答                  |
| DeepSeek Anthropic API | 真实 `/v1/messages` 非流式/流式调用、工具调用与最终回答               |

## 运行

```bash
# 1. 准备环境变量
cp examples/deepseek-agent/.env.example examples/deepseek-agent/.env
# 编辑 examples/deepseek-agent/.env，填入真实的 DEEPSEEK_API_KEY 或 KURUMI_API_KEY

# 2. 构建基础包并运行真实调用
pnpm --filter @kurumi/example-deepseek-agent verify

# 3. 使用 DeepSeek 的 Anthropic 兼容 API 验证 @kurumi/anthropic
pnpm --filter @kurumi/example-deepseek-agent verify:anthropic
```

`verify` 与 `verify:anthropic` 的 pre 脚本会自动按依赖顺序构建 `@kurumi/core`、`@kurumi/schema`、`@kurumi/config`、`@kurumi/log`、`@kurumi/agent`、`@kurumi/open-ai`、`@kurumi/anthropic`。

也可以手动运行：

```bash
pnpm --filter @kurumi/open-ai build
pnpm --filter @kurumi/anthropic build
cd examples/deepseek-agent
node --env-file=.env src/verify.ts                  # OpenAI 协议
node --env-file=.env src/verify.ts agent.anthropic.jsonc  # Anthropic 协议
```

## 配置

`examples/deepseek-agent/agent.jsonc`（OpenAI 协议）：

```jsonc
{
  "api_key": "{env:DEEPSEEK_API_KEY}",
  "api_format": "openai",
  "model": "deepseek-chat",
  "base_url": "https://api.deepseek.com",
  "temperature": 0.2,
  "max_steps": 4,
  "max_output_tokens": 1024,
  "thinking": false,
  "thinking_budget_tokens": 2048,
  "reasoning_effort": "medium",
  "log_level": "info",
  "system": "你是 Kurumi Agent SDK 的验证助手。",
}
```

`api_key` 使用 `@kurumi/config` 的 `{env:DEEPSEEK_API_KEY}` 插值；`KURUMI_API_KEY` 由 envSource 映射为 `api_key`，优先级更高。若在配置文件中删除 `api_key`，则需要提供拼写正确的 `KURUMI_API_KEY`（envSource 只映射该前缀）。

`examples/deepseek-agent/agent.anthropic.jsonc`（Anthropic 协议，复用同一份 `verify.ts`）：

```jsonc
{
  "api_key": "{env:DEEPSEEK_API_KEY}",
  "api_format": "anthropic",
  "model": "deepseek-flash",
  "base_url": "https://api.deepseek.com/anthropic",
  "max_output_tokens": 2048,
  "thinking": true,
  "thinking_budget_tokens": 1024,
  "reasoning_effort": "high",
  // 其余字段与 agent.jsonc 相同
}
```

环境变量（`KURUMI_*` 优先级高于配置文件）：

| 变量                            | 必填 | 说明                                             |
| ------------------------------- | ---- | ------------------------------------------------ |
| `DEEPSEEK_API_KEY`              | 是\* | 由 `{env:DEEPSEEK_API_KEY}` 插值读取             |
| `KURUMI_API_KEY`                | 是\* | envSource 映射为 `api_key`，优先级更高           |
| `KURUMI_API_FORMAT`             | 否   | `openai` / `anthropic`                           |
| `KURUMI_MODEL`                  | 否   | 覆盖模型，例如 `deepseek-flash`                  |
| `KURUMI_BASE_URL`               | 否   | 覆盖 API 根地址，例如 `https://api.deepseek.com` |
| `KURUMI_THINKING`               | 否   | `true` / `false`，覆盖 thinking 开关             |
| `KURUMI_THINKING_BUDGET_TOKENS` | 否   | 思考预算 Token 数                                |
| `KURUMI_REASONING_EFFORT`       | 否   | `minimal` / `low` / `medium` / `high`            |
| `KURUMI_LOG_LEVEL`              | 否   | `trace` / `debug` / `info` / `warn` / `error`    |
| `KURUMI_MAX_STEPS`              | 否   | 覆盖最大模型轮数（1–16）                         |

\* `DEEPSEEK_API_KEY` 与 `KURUMI_API_KEY` 至少提供一个。

配置由 `@kurumi/config` 的 `fileSource` + `envSource` 合并，并由 schema 校验；`api_key` 使用 `secret()`，日志只输出 `[REDACTED]`。

## 验证流程

1. 加载 `.env` 与 `agent.jsonc`（或命令行指定的 `agent.anthropic.jsonc`），校验配置并打印脱敏配置；
2. 创建 `@kurumi/log` 日志器；
3. 按 `api_format` 选择 `@kurumi/open-ai` 或 `@kurumi/anthropic` 创建提供方（`id: deepseek`）；
4. 先执行一次非流式 `generate`，验证基础消息与 usage；
5. 创建两个工具：`get_current_time`、`calculate`；
6. 流式运行 Agent，打印事件与工具审批；
7. 断言：
   - Agent 以 `completed` 结束；
   - 模型真实调用了两个工具；
   - 工具结果无错误；
   - 最终回答非空；
   - 启用 thinking 但模型未返回推理时记录告警，不判定验证失败；
8. 打印累计 usage、步骤数、重试次数、推理文本与最终回答。

预期结尾类似：

```text
INFO 工具结束 ... tool="get_current_time" isError=false
INFO 工具结束 ... tool="calculate" isError=false
INFO Agent 结束 ... stopReason="completed" steps=2
INFO ✅ 验证通过 ... toolCalls=["get_current_time","calculate"]
```

## Thinking

`@kurumi/agent` 的 `thinking` 配置会随 `ModelRequest` 传给适配器：

```jsonc
{
  "thinking": true,
  "thinking_budget_tokens": 2048,
  "reasoning_effort": "high",
}
```

- `@kurumi/anthropic` 映射为 `thinking: { type: "enabled", budget_tokens }`；关闭时映射为 `{ type: "disabled" }`。
- `@kurumi/open-ai` 映射 `isEnabled` 为 `thinking: { type: "enabled" | "disabled" }`，`reasoning_effort` 原样传递；官方 OpenAI 可设置 provider 选项 `thinkingField: false` 只发送 `reasoning_effort`。
- 流式推理增量通过 `model_delta.delta.type === 'reasoning_delta'` 暴露；最终消息可用 `extractMessageReasoning(message)` 提取。
- Anthropic 的 `thinking.signature` / `redacted_thinking.data` 会保存在 `reasoning` 内容块中并在后续工具轮次原样回传。

## 切换 Anthropic 协议

`verify:anthropic` 不复用逻辑副本，而是给同一份 `src/verify.ts` 传入
`agent.anthropic.jsonc`：

```bash
pnpm --filter @kurumi/example-deepseek-agent verify:anthropic
```

它会用 DeepSeek 的 Anthropic 兼容端点（`https://api.deepseek.com/anthropic`，
模型 `deepseek-flash`）执行与 OpenAI 协议完全相同的验证流程和断言，只是
provider 由 `@kurumi/open-ai` 换成 `@kurumi/anthropic`。

## 安全

- `examples/deepseek-agent/.env` 已被 `.gitignore` 忽略；不要提交真实 Key。
- 示例不会打印 Key；配置日志使用 `schema.redact()`。
- 适配器只把 Key 放在请求头，不写入错误消息。
- 运行超时：配置加载 10 秒，Agent 运行 90 秒；工具单次超时 10 秒。

## 故障排查

| 现象                      | 处理                                                                        |
| ------------------------- | --------------------------------------------------------------------------- |
| `api_key` schema 校验失败 | 确认 `.env` 中有 `DEEPSEEK_API_KEY=...` 或 `KURUMI_API_KEY=...`（注意拼写） |
| `DeepSeek HTTP 401`       | Key 无效或已过期                                                            |
| `DeepSeek HTTP 429`       | 触发限流，稍后重试                                                          |
| 模型未调用工具            | 检查模型是否支持 function calling；可换 `deepseek-chat`                     |
| `EACCES` / 找不到模块     | 先在仓库根执行 `pnpm install`，再运行 `verify`                              |
