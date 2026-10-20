# @kurumi/open-ai 形式化契约

> 版本：0.1.0  
> 范围：`providers/open-ai/src`（选项、Chat Completions 请求/响应映射、SSE 流解析、错误）。  
> 判定：模型适配器是 Agent SDK 的对外协议边界，直接处理不可信网络数据与 API Key，属于对外契约类模块；**不适用** FSVDD §1.2 的纯文档/构建配置豁免。

## 1. 职责边界

`@kurumi/open-ai` 负责：

- 把 `ModelRequest` 确定性映射为 OpenAI Chat Completions 请求；
- 把非流式 JSON/SSE 流映射为 `ModelResponse`/`ModelStreamEvent`；
- 校验响应结构、工具参数、usage 与流事件；
- 提供可注入 `fetch`、自定义 headers、`maxTokensField`、推理字段名等协议扩展；
- 将所有失败（含 fetch/网络、读取流、SSE error 负载）包装为 `OpenAIProviderError`，保留 `providerId`、HTTP `status` 与 `cause`。

不负责：

- Agent 循环、工具执行、重试、取消（由 `@kurumi/agent` 负责）；
- 配置加载、日志、指标；
- 非 Chat Completions 协议（Responses API、Anthropic Messages 等）；
- 多模态之外的 vendor 专属请求结构（通过 `providerOptions` 透传）。

## 2. 不变量

| 编号           | 不变量                                                                                                                                                  |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `I-OPENAI-001` | 主入口及其非测试源码不得导入 `node:`、`@kurumi/config`、`@kurumi/log`，不得直接调用 `console.*`。                                                       |
| `I-OPENAI-002` | API Key 只能出现在 `Authorization` 请求头；不得出现在错误消息、事件或日志字段中。                                                                       |
| `I-OPENAI-003` | 不修改调用方传入的 `ModelRequest`、消息、工具或选项；请求体由新对象组成。                                                                               |
| `I-OPENAI-004` | `providerOptions` 只能补充 vendor 字段，不能覆盖 `model`、`messages`、`stream`、`stream_options`、`tools`、`tool_choice`、`temperature`、`max_*`。      |
| `I-OPENAI-005` | 所有响应/流事件在映射前必须校验；SSE `error` 负载、非法 choices/delta、非法结构必须抛 `OpenAIProviderError`，不得静默忽略。                             |
| `I-OPENAI-006` | 工具参数必须是合法 JSON；空字符串按 `{}` 处理；危险键由 Agent 的消息校验拒绝。                                                                          |
| `I-OPENAI-007` | 流式响应必须最终产生且仅产生一个 `response` 事件；即使缺少 `[DONE]`，正常 EOF 也必须完成组装。                                                          |
| `I-OPENAI-008` | 适配器无全局可变状态；同一个 provider 可被并发调用。                                                                                                    |
| `I-OPENAI-009` | 核心 `request.thinking` 已配置时，适配器拥有 `reasoning_effort` 与思考开关字段的映射权，`providerOptions` 不得覆盖；缺省时 `providerOptions` 原样透传。 |

## 3. 前置约束

| 编号                | 约束                                                                                                                                               | 失败语义              |
| ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------- |
| `FC-OPENAI-PRE-001` | `options` 必须是对象；`apiKey` 非空；`id`、`baseUrl`、`reasoningContentField` 若提供必须非空；`baseUrl` 必须是合法 http(s) URL 且不含 query/hash。 | `OpenAIProviderError` |
| `FC-OPENAI-PRE-002` | `baseUrl` 末尾 `/` 必须去除；完整 URL 为 `${baseUrl}/chat/completions`。                                                                           | 构造期规范化          |
| `FC-OPENAI-PRE-003` | `headers` 若提供必须是普通对象且所有值为字符串；自定义头不能覆盖 content-type/authorization。                                                      | `OpenAIProviderError` |
| `FC-OPENAI-PRE-004` | `fetch` 若提供必须是函数；`maxTokensField ∈ {max_tokens, max_completion_tokens}`；`shouldIncludeStreamUsage` 必须是布尔值。                        | `OpenAIProviderError` |
| `FC-OPENAI-PRE-005` | 请求消息必须已通过 `@kurumi/agent` 校验；适配器只接受 system/user/assistant/tool 四类，非法角色抛 `OpenAIProviderError`。                          | `OpenAIProviderError` |
| `FC-OPENAI-PRE-006` | 流式 delta 的 `content` 必须是字符串或 null；`tool_calls` 必须是数组；index 非负安全整数；id/name 非空；arguments 必须是字符串。                   | `OpenAIProviderError` |
| `FC-OPENAI-PRE-007` | 非流式响应必须包含 `choices[0].message`；usage 计数必须是非负安全整数。                                                                            | `OpenAIProviderError` |
| `FC-OPENAI-PRE-008` | `thinkingField` 若提供必须是 `false` 或非空字符串；字符串不得是受保护请求字段或 `reasoning_effort`。                                               | `OpenAIProviderError` |

## 4. 后置约束

| 编号                 | 约束                                                                                                                                                                       |
| -------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `FC-OPENAI-POST-001` | 请求包含 `model`、映射后的 `messages`、`stream`；有工具时包含 `tools` 与可选 `tool_choice`；`temperature`/`maxOutputTokens` 按配置映射。                                   |
| `FC-OPENAI-POST-002` | user 纯文本使用字符串；含图片时使用 `text` + `image_url` 内容数组，data URL 为 `data:${mediaType};base64,${data}`。                                                        |
| `FC-OPENAI-POST-003` | assistant 的 reasoning 内容块不回传；有工具调用且无文本时 `content=null`；工具调用参数使用 `JSON.stringify(input)`。                                                       |
| `FC-OPENAI-POST-004` | tool 消息映射为 `{ role: 'tool', tool_call_id, name, content }`。                                                                                                          |
| `FC-OPENAI-POST-005` | 非流式响应映射为文本、推理、工具调用内容块；空响应生成 `text: ''`；工具参数空字符串映射为 `{}`。                                                                           |
| `FC-OPENAI-POST-006` | `finish_reason` 按契约映射；存在工具调用时 `tool_calls` 优先于 `stop`。                                                                                                    |
| `FC-OPENAI-POST-007` | usage 映射 `prompt_tokens`/`completion_tokens`，缓存优先 `prompt_tokens_details.cached_tokens`，其次 `prompt_cache_hit_tokens`；返回冻结 TokenUsage。                      |
| `FC-OPENAI-POST-008` | 流式按 `data:` 行解析；支持 `[DONE]`、空行、`\r\n`、缺失 `[DONE]` 的 EOF；只忽略带 usage 的无 choices chunk；SSE `error`、非对象 choice/delta 必须抛错；单事件缓冲有上限。 |
| `FC-OPENAI-POST-009` | 流式增量按文本、推理、工具参数顺序转发；工具参数跨 chunk 累积；最终 response 与非流式拥有相同的消息/停止原因/usage 契约。                                                  |
| `FC-OPENAI-POST-010` | HTTP 非 2xx 抛 `OpenAIProviderError`，带 `status` 与截断到 1000 字符的响应体；错误体读取有字节上限；不包含 Authorization。                                                 |
| `FC-OPENAI-POST-011` | 非 JSON 响应、SSE JSON/error 非法、读取流失败、缺少 body、消息/usage/工具参数非法、网络请求失败均抛 `OpenAIProviderError` 并保留 `cause`（若有）。                         |
| `FC-OPENAI-POST-012` | `request.thinking` 映射：`effort` → `reasoning_effort`；`isEnabled` → `{[thinkingField]: {type:'enabled'                                                                   | 'disabled'}}`（`thinkingField=false` 时不发送）；`budgetTokens`被忽略（OpenAI 协议无预算字段）。未配置时保留`providerOptions` 透传。 |

## 5. 流状态机

```text
States  = { Connecting, Streaming, Assemble, Done }
Initial = Connecting
Final   = { Done }
```

| 当前状态     | 事件           | 下一状态    | 副作用 / 拦截                      |
| ------------ | -------------- | ----------- | ---------------------------------- |
| `Connecting` | HTTP 2xx       | `Streaming` | 获取 body reader                   |
| `Connecting` | HTTP 非 2xx    | —           | 抛 `OpenAIProviderError`           |
| `Connecting` | 无 body        | —           | 抛 `OpenAIProviderError`           |
| `Streaming`  | data delta     | `Streaming` | 转发 text/reasoning/tool_call 增量 |
| `Streaming`  | usage chunk    | `Streaming` | 记录 usage                         |
| `Streaming`  | `[DONE]` / EOF | `Assemble`  | 停止读流                           |
| `Streaming`  | 非法 JSON/字段 | —           | 抛 `OpenAIProviderError`           |
| `Assemble`   | 组装成功       | `Done`      | 发出唯一 `response` 事件           |

## 6. 错误分类

| 类型 / code                                      | 触发条件                                                     | 上下文                            |
| ------------------------------------------------ | ------------------------------------------------------------ | --------------------------------- |
| `OpenAIProviderError` / `open_ai_provider_error` | 选项非法、HTTP 非 2xx、响应/流/usage/工具参数非法、body 缺失 | `providerId`、`status?`、`cause?` |
| `AgentModelError`（上游）                        | `@kurumi/agent` 捕获适配器错误后包装                         | `providerId`、`cause`             |

## 7. 边界采样

- URL：默认根地址、带尾斜杠、自定义路径、空字符串；
- 消息：纯文本、文本+图片、reasoning、仅工具调用、tool 结果；
- 工具：空参数、合法 JSON、非法 JSON、缺失 id/name、跨 chunk 参数；
- usage：缺省、null、缓存字段两种形态、负数、小数、溢出；
- 思考：`thinking` 缺省、启用/关闭、effort、`providerOptions` 同时提供 `thinking`/`reasoning_effort`、`thinkingField=false`；
- 流：`[DONE]`、无 `[DONE]`、空行、`\r\n`、空 choices、未知字段、非法 JSON、非 2xx、无 body；
- 安全：API Key 不出现在错误消息、headers 不能覆盖 Authorization。

## 8. 追溯矩阵

| 约束                                                          | 测试文件 / 用例                                                             |
| ------------------------------------------------------------- | --------------------------------------------------------------------------- |
| `PRE-001` / `PRE-002` / `PRE-004`                             | `src/provider.test.ts` · 选项默认值、自定义根地址、非法选项                 |
| `PRE-003`                                                     | `src/provider.test.ts` · headers 合并与非法 headers                         |
| `PRE-005` / `POST-001` / `POST-002` / `POST-003` / `POST-004` | `src/provider.test.ts` · 非流式请求映射（消息/图片/工具）                   |
| `PRE-007` / `POST-005` / `POST-006` / `POST-007`              | `src/provider.test.ts` · 响应映射、停止原因、缓存 usage、非法响应           |
| `POST-010` / `I-OPENAI-002`                                   | `src/provider.test.ts` · HTTP 401、错误不泄漏 Key                           |
| `PRE-006` / `POST-008` / `POST-009`                           | `src/stream.test.ts` · 增量转发、工具累积、缺失 DONE、非法 delta、SSE error |
| `POST-011` 传输/读取错误                                      | `src/provider.test.ts`、`src/stream.test.ts` · 网络错误、reader 错误包装    |
| `POST-011`                                                    | `src/provider.test.ts`、`src/stream.test.ts` · 非 JSON/SSE JSON/无 body     |
| `I-OPENAI-001`                                                | `src/architecture.test.ts` · 无 node/config/log/console 依赖                |
| `I-OPENAI-004`                                                | `src/provider.test.ts` · providerOptions 合并、核心字段不可覆盖             |
| `I-OPENAI-007`                                                | `src/stream.test.ts` · 最终 response 事件                                   |
| `PRE-008` / `POST-012` / `I-OPENAI-009`                       | `src/provider.test.ts` · thinking/effort 映射、字段保护与关闭开关           |

## 9. 兼容性与演进

- 破坏性变更：调整 `OpenAIProviderOptions` 必选字段、默认根地址、`OpenAIProviderError` code、消息/工具映射语义、流事件顺序。
- 非破坏性变更：新增可选选项（必须提供默认值）、新增可选响应字段、扩展 usage 缓存映射、新增 vendor 推理字段别名。
- 新增 provider 子包应复用 `@kurumi/agent` 的 `ModelProvider` 契约，不得把 vendor DTO 写入 Agent 核心。
- 修改公开 API 必须同步更新本契约、README、示例与测试。
