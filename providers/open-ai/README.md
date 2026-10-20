# @kurumi/open-ai

OpenAI 协议兼容的 `ModelProvider` 适配器：**Chat Completions + 工具调用 + SSE 流式 + usage 映射**。

适用于 OpenAI 以及兼容 Chat Completions 协议的服务（DeepSeek、OpenRouter、Groq、vLLM、Ollama 等）。

- **通用协议**：消息、工具、`tool_choice`、`finish_reason`、usage 全部按 OpenAI 协议映射。
- **流式优先**：实现 `stream`，转发文本、可配置推理字段与工具参数增量。
- **扩展可控**：`providerOptions` 透传 vendor 字段；`maxTokensField`、`reasoningContentField`、`shouldIncludeStreamUsage` 可按服务调整。
- **不写日志**：错误带 `providerId` 与 HTTP `status`，由 `@kurumi/agent` 包装为 `AgentModelError`。
- **零 Node 依赖**：使用全局 `fetch`/`Response`/`TextDecoder`，可注入 fetch 便于测试或代理。

## 安装与导入

```ts
import { createOpenAIProvider } from '@kurumi/open-ai';
```

## 快速开始

```ts
import { createAgent, createUserMessage } from '@kurumi/agent';
import { createOpenAIProvider } from '@kurumi/open-ai';

const apiKey = process.env.KURUMI_API_KEY;
if (apiKey === undefined || apiKey.trim() === '') throw new Error('KURUMI_API_KEY is required');

const agent = createAgent({
  provider: createOpenAIProvider({
    apiKey,
    baseUrl: 'https://api.deepseek.com',
    id: 'deepseek',
  }),
  model: 'deepseek-chat',
});

const result = await agent.run([createUserMessage('你好')]);
```

默认根地址 `https://api.openai.com/v1`，实际请求 `${baseUrl}/chat/completions`。

## 选项

| 选项                       | 默认值                      | 说明                                            |
| -------------------------- | --------------------------- | ----------------------------------------------- |
| `apiKey`                   | 必填                        | 写入 `Authorization: Bearer ...`                |
| `baseUrl`                  | `https://api.openai.com/v1` | API 根地址，自动去除末尾 `/`                    |
| `id`                       | `open-ai`                   | 提供方 ID，用于错误与事件                       |
| `headers`                  | `{}`                        | 追加请求头；不能覆盖 content-type/authorization |
| `fetch`                    | 全局 `fetch`                | 测试、代理或自定义传输                          |
| `maxTokensField`           | `max_tokens`                | 可切换为 `max_completion_tokens`                |
| `shouldIncludeStreamUsage` | `true`                      | 流式请求发送 `stream_options.include_usage`     |
| `reasoningContentField`    | `reasoning_content`         | 推理内容字段名（DeepSeek 等）                   |
| `thinkingField`            | `thinking`                  | 思考开关字段名；`false` 表示不发送开关字段      |

## 协议映射

### 请求

- `system` → 字符串内容；
- `user` 纯文本 → 字符串；含图片 → `text` / `image_url` 内容数组（`data:image/...;base64,...`）；
- `assistant` → `content` + `tool_calls`；`reasoning` 内容块不回传（避免部分服务拒绝）；
- `tool` → `{ role: 'tool', tool_call_id, name, content }`；
- `tools` → `{ type: 'function', function: { name, description, parameters } }`；
- `toolChoice`：
  - `auto` / `none` / `required` 原样传递；
  - `{ type: 'tool', name }` → `{ type: 'function', function: { name } }`；
- `thinking.effort` → `reasoning_effort`；`thinking.isEnabled` → `{[thinkingField]: { type: 'enabled' | 'disabled' }}`；
- `thinkingField=false` 时不发送思考开关，仅发送 `reasoning_effort`（适配官方 OpenAI）；
- `budgetTokens` 被忽略（Chat Completions 无预算字段）；
- `providerOptions` 会合并进请求体，但不能覆盖 `model`/`messages`/`stream`/`tools`/`temperature`/`max_*` 等核心字段；核心 `thinking` 优先于 `providerOptions` 中的 `thinking`/`reasoning_effort`。

### 响应

- `message.content` → `text` 内容块；
- `message.reasoning_content`（或自定义字段）→ `reasoning` 内容块；
- `message.tool_calls` → `tool_call` 内容块；参数按 JSON 解析，空字符串视为 `{}`；
- `finish_reason`：
  - `stop` → `end`（若存在工具调用则为 `tool_calls`）；
  - `length` → `max_tokens`；
  - `content_filter` → `content_filter`；
  - `tool_calls` → `tool_calls`；
  - 其他 → `other`（存在工具调用时为 `tool_calls`）；
- usage：
  - `prompt_tokens` → `inputTokens`；
  - `completion_tokens` → `outputTokens`；
  - `prompt_tokens_details.cached_tokens` 或 `prompt_cache_hit_tokens` → `cacheReadTokens`。

### 流式

支持以下 SSE `data:` 事件：

- `choices[0].delta.content` → `text_delta`；
- `choices[0].delta[reasoningContentField]` → `reasoning_delta`；
- `choices[0].delta.tool_calls[]` → `tool_call_delta`（累积 id/name/arguments）；
- `usage` → 最终响应 usage；
- `[DONE]` 或流结束 → 组装最终 `response` 事件。

## 错误

```ts
import { OpenAIProviderError } from '@kurumi/open-ai';

try {
  await provider.generate(request, options);
} catch (error) {
  if (error instanceof OpenAIProviderError) {
    error.code; // 'open_ai_provider_error'
    error.providerId; // 'open-ai'
    error.status; // 401 / 429 / ...
    error.cause;
  }
}
```

HTTP 错误信息包含状态码与截断后的响应体，**不包含 API Key**。

## 脚本

```bash
pnpm --filter @kurumi/open-ai typecheck
pnpm --filter @kurumi/open-ai test
pnpm --filter @kurumi/open-ai test:coverage
pnpm --filter @kurumi/open-ai build
```

契约与追溯矩阵：[`docs/spec/open-ai.md`](../../docs/spec/open-ai.md)

真实 DeepSeek 验证：[`examples/deepseek-agent`](../../examples/deepseek-agent/README.md)
