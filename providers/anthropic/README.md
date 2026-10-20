# @kurumi/anthropic

Anthropic Messages 协议兼容的 `ModelProvider` 适配器：**Messages API + 工具调用 + 扩展思考 + SSE 流式 + usage 映射**。

适用于 Anthropic 官方 API 以及兼容 Messages 协议的服务（DeepSeek Anthropic API、Claude Code 兼容网关等）。

- **通用协议**：system、user/assistant 消息、图片、工具、`tool_choice`、`stop_reason`、usage 全部按 Anthropic Messages 协议映射。
- **工具结果合并**：连续 tool 结果自动合并为一条 user 消息，符合 Anthropic 的 `tool_result` 结构。
- **流式优先**：实现 `stream`，转发 text、thinking 与 `input_json_delta` 工具参数增量。
- **扩展可控**：`providerOptions` 透传 vendor 字段；`anthropicVersion`、`defaultMaxTokens`、headers 可配置。
- **不写日志**：错误带 `providerId` 与 HTTP `status`，由 `@kurumi/agent` 包装为 `AgentModelError`。
- **零 Node 依赖**：使用全局 `fetch`/`Response`/`TextDecoder`，可注入 fetch 便于测试或代理。

## 安装与导入

```ts
import { createAnthropicProvider } from '@kurumi/anthropic';
```

## 快速开始

```ts
import { createAgent, createUserMessage } from '@kurumi/agent';
import { createAnthropicProvider } from '@kurumi/anthropic';

const apiKey = process.env.KURUMI_API_KEY;
if (apiKey === undefined || apiKey.trim() === '') throw new Error('KURUMI_API_KEY is required');

const agent = createAgent({
  provider: createAnthropicProvider({
    id: 'deepseek-anthropic',
    apiKey,
    baseUrl: 'https://api.deepseek.com/anthropic',
  }),
  model: 'deepseek-flash',
  maxOutputTokens: 1_024,
});

const result = await agent.run([createUserMessage('你好')]);
```

默认根地址 `https://api.anthropic.com`，实际请求 `${baseUrl}/v1/messages`。Anthropic 官方 SDK 的 `base_url` 同样不包含 `/v1`。

## 选项

| 选项                          | 默认值                      | 说明                                                          |
| ----------------------------- | --------------------------- | ------------------------------------------------------------- |
| `apiKey`                      | 必填                        | 写入 `x-api-key`                                              |
| `baseUrl`                     | `https://api.anthropic.com` | API 根地址，自动去除末尾 `/`                                  |
| `id`                          | `anthropic`                 | 提供方 ID，用于错误与事件                                     |
| `headers`                     | `{}`                        | 追加请求头；不能覆盖 content-type/x-api-key/anthropic-version |
| `fetch`                       | 全局 `fetch`                | 测试、代理或自定义传输                                        |
| `anthropicVersion`            | `2023-06-01`                | `anthropic-version` 请求头                                    |
| `defaultMaxTokens`            | `4096`                      | 未提供 `maxOutputTokens` 时使用的 `max_tokens`（协议必填）    |
| `defaultThinkingBudgetTokens` | `2048`                      | 启用 thinking 且未提供 `budgetTokens` 时使用的预算            |

## 协议映射

### 请求

- 所有 `system` 消息合并为顶层 `system` 字符串；
- `user` 纯文本 → 字符串；含图片 → `text` / `image` 内容块（`source.type = "base64"`）；
- `assistant` → `text` + `tool_use`；`reasoning` 带 `signature` 时回传 `thinking` 块，带 `redactedData` 时回传 `redacted_thinking` 块，两者皆无时跳过；
- `tool` → user 消息中的 `tool_result` 块；连续 tool 结果合并到同一条 user 消息；
- `tools` → `{ name, description, input_schema }`；
- `toolChoice`：
  - `auto` → `{ type: 'auto' }`；
  - `none` → `{ type: 'none' }`；
  - `required` → `{ type: 'any' }`；
  - `{ type: 'tool', name }` → `{ type: 'tool', name }`；
- `max_tokens` 始终发送：取 `maxOutputTokens`，缺省 `defaultMaxTokens`；
- `thinking`：`{ isEnabled: false }` → `{ type: 'disabled' }`；启用时 → `{ type: 'enabled', budget_tokens }`，预算取 `budgetTokens`，缺省 `defaultThinkingBudgetTokens`；
- `providerOptions` 会合并进请求体，但不能覆盖 `model`/`messages`/`system`/`max_tokens`/`stream`/`tools`/`tool_choice`/`temperature`；核心 `thinking` 优先于 `providerOptions.thinking`。

### 响应

- `content[].type = "text"` → `text` 内容块；
- `content[].type = "thinking"` → 带可选 `signature` 的 `reasoning` 内容块；
- `content[].type = "redacted_thinking"` → 带 `redactedData` 的 `reasoning` 内容块；
- `content[].type = "tool_use"` → `tool_call` 内容块；`input` 必须是 JSON 安全值；
- 内容块按协议顺序映射；没有内容块时生成 `text: ''`；
- `stop_reason`：
  - `end_turn` / `stop_sequence` → `end`（存在工具调用时为 `tool_calls`）；
  - `max_tokens` → `max_tokens`；
  - `tool_use` → `tool_calls`；
  - `refusal` → `content_filter`；
  - 其他/缺省 → `other`（存在工具调用时为 `tool_calls`）；
- usage：
  - `input_tokens + cache_creation_input_tokens + cache_read_input_tokens` → `inputTokens`；
  - `output_tokens` → `outputTokens`；
  - `cache_read_input_tokens` → `cacheReadTokens`；
  - `cache_creation_input_tokens` → `cacheWriteTokens`。

### 流式

支持以下 SSE `data:` 事件：

- `message_start` → 记录输入 usage；
- `content_block_start`：
  - `text` / `thinking` 起始内容 → `text_delta` / `reasoning_delta`（非空时）；
  - `redacted_thinking` → 记录加密数据，最终映射为 `reasoning`；
  - `tool_use` → `tool_call_delta`（携带 id/name 与空参数）；
- `content_block_delta`：
  - `text_delta` → `text_delta`；
  - `thinking_delta` → `reasoning_delta`；
  - `input_json_delta` → `tool_call_delta`；
  - `signature_delta` → 累积到最终 `reasoning.signature`；
- `message_delta` → 更新 `stop_reason` 与 usage；
- `ping` → 忽略；
- `message_stop` 或正常 EOF → 组装最终 `response` 事件。

## 错误

```ts
import { AnthropicProviderError } from '@kurumi/anthropic';

try {
  await provider.generate(request, options);
} catch (error) {
  if (error instanceof AnthropicProviderError) {
    error.code; // 'anthropic_provider_error'
    error.providerId; // 'anthropic'
    error.status; // 401 / 429 / ...
    error.cause;
  }
}
```

HTTP 错误信息包含状态码与截断后的响应体，**不包含 API Key**。

## 脚本

```bash
pnpm --filter @kurumi/anthropic typecheck
pnpm --filter @kurumi/anthropic test
pnpm --filter @kurumi/anthropic test:coverage
pnpm --filter @kurumi/anthropic build
```

契约与追溯矩阵：[`docs/spec/anthropic.md`](../../docs/spec/anthropic.md)

真实 DeepSeek Anthropic API 验证：[`examples/deepseek-agent`](../../examples/deepseek-agent/README.md)（`verify:anthropic`）。
