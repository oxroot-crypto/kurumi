# @kurumi/anthropic 形式化契约

> 版本：0.1.0  
> 范围：`providers/anthropic/src`（选项、Anthropic Messages 请求/响应映射、SSE 流解析、错误）。  
> 判定：模型适配器是 Agent SDK 的对外协议边界，直接处理不可信网络数据与 API Key，属于对外契约类模块；**不适用** FSVDD §1.2 的纯文档/构建配置豁免。

## 1. 职责边界

`@kurumi/anthropic` 负责：

- 把 `ModelRequest` 确定性映射为 Anthropic Messages 请求；
- 把非流式 JSON 与 SSE 流映射为 `ModelResponse`/`ModelStreamEvent`；
- 校验响应结构、内容块、工具输入、usage 与流事件；
- 提供可注入 `fetch`、自定义 headers、`anthropic-version`、默认 `max_tokens` 等协议扩展；
- 将所有失败（含 fetch/网络、读取流、SSE error 负载）包装为 `AnthropicProviderError`，保留 `providerId`、HTTP `status` 与 `cause`。

不负责：

- Agent 循环、工具执行、重试、取消（由 `@kurumi/agent` 负责）；
- 配置加载、日志、指标；
- OpenAI Chat Completions / Responses API；
- Anthropic Files、Batch、Token Counting 等非 Messages 端点；
- 扩展思考签名的加密/解密：适配器只原样保存与回传 `signature` 与 `redacted_thinking.data`，不解析其内容。

## 2. 不变量

| 编号              | 不变量                                                                                                                                                    |
| ----------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `I-ANTHROPIC-001` | 主入口及其非测试源码不得导入 `node:`、`@kurumi/config`、`@kurumi/log`，不得直接调用 `console.*`。                                                         |
| `I-ANTHROPIC-002` | API Key 只能出现在 `x-api-key` 请求头；不得出现在错误消息、事件或日志字段中。                                                                             |
| `I-ANTHROPIC-003` | 不修改调用方传入的 `ModelRequest`、消息、工具或选项；请求体由新对象组成。                                                                                 |
| `I-ANTHROPIC-004` | `providerOptions` 只能补充 vendor 字段，不能覆盖 `model`、`messages`、`system`、`max_tokens`、`stream`、`tools`、`tool_choice`、`temperature`。           |
| `I-ANTHROPIC-005` | 所有响应/流事件在映射前必须校验；SSE `error`、非法内容块/delta、非法结构必须抛 `AnthropicProviderError`，不得静默忽略。                                   |
| `I-ANTHROPIC-006` | 工具输入必须是 JSON 安全值；流式 `tool_use` 参数跨 `input_json_delta` 累积后必须是合法 JSON；空字符串按 `{}` 处理。                                       |
| `I-ANTHROPIC-007` | 流式响应必须最终产生且仅产生一个 `response` 事件；即使缺少 `message_stop`，正常 EOF 也必须完成组装。                                                      |
| `I-ANTHROPIC-008` | 适配器无全局可变状态；同一个 provider 可被并发调用。                                                                                                      |
| `I-ANTHROPIC-009` | 响应内容块按协议顺序映射为 Agent 内容块；`thinking` 映射为带可选 `signature` 的 `reasoning`，`redacted_thinking` 映射为带 `redactedData` 的 `reasoning`。 |
| `I-ANTHROPIC-010` | assistant 的 `reasoning` 只有携带 `signature` 或 `redactedData` 时才回传；缺少续传数据时跳过，避免伪造无效 thinking 块。                                  |
| `I-ANTHROPIC-011` | 核心 `request.thinking` 已配置时，其映射结果优先于 `providerOptions.thinking`；缺省时仍允许 `providerOptions` 提供 vendor 字段。                          |

## 3. 前置约束

| 编号                   | 约束                                                                                                                                                                 | 失败语义                 |
| ---------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------ |
| `FC-ANTHROPIC-PRE-001` | `options` 必须是对象；`apiKey` 非空；`id`、`baseUrl`、`anthropicVersion` 若提供必须非空；`baseUrl` 必须是合法 http(s) URL 且不含 query/hash。                        | `AnthropicProviderError` |
| `FC-ANTHROPIC-PRE-002` | `baseUrl` 末尾 `/` 必须去除；完整 URL 为 `${baseUrl}/v1/messages`。                                                                                                  | 构造期规范化             |
| `FC-ANTHROPIC-PRE-003` | `headers` 若提供必须是普通对象且所有值为字符串；自定义头不能覆盖 `content-type`、`x-api-key`、`anthropic-version`。                                                  | `AnthropicProviderError` |
| `FC-ANTHROPIC-PRE-004` | `fetch` 若提供必须是函数；`defaultMaxTokens` 必须是正安全整数。                                                                                                      | `AnthropicProviderError` |
| `FC-ANTHROPIC-PRE-005` | `request.model` 必须非空；`messages` 必须非空且角色只允许 system/user/assistant/tool；`maxOutputTokens` 若提供必须是正安全整数；`temperature` 若提供必须是有限数值。 | `AnthropicProviderError` |
| `FC-ANTHROPIC-PRE-006` | 非流式响应必须是普通对象，`content` 必须是数组；`role` 若存在必须为 `assistant`；`type` 若存在必须为 `message`。                                                     | `AnthropicProviderError` |
| `FC-ANTHROPIC-PRE-007` | 响应内容块只允许 `text`（`text` 字符串）、`thinking`（`thinking` 字符串）、`tool_use`（非空 id/name、JSON 安全 input）；其他类型必须显式失败。                       | `AnthropicProviderError` |
| `FC-ANTHROPIC-PRE-008` | usage 各计数若存在且非 null 必须是非负安全整数；`input_tokens + cache_creation_input_tokens + cache_read_input_tokens` 不得溢出安全整数。                            | `AnthropicProviderError` |
| `FC-ANTHROPIC-PRE-009` | 流事件必须是普通对象且 `type` 为已知值；`index` 必须是非负安全整数；delta 类型必须与内容块类型匹配。                                                                 |
| `FC-ANTHROPIC-PRE-010` | `request.thinking` 若提供必须已由 Agent 归一化；`defaultThinkingBudgetTokens` 必须是正安全整数。                                                                     |
| `FC-ANTHROPIC-PRE-011` | 响应 `thinking.signature`、`redacted_thinking.data` 若存在必须是非空字符串；流式 `signature_delta.signature` 必须是字符串。                                          | `AnthropicProviderError` |

## 4. 后置约束

| 编号                    | 约束                                                                                                                                                                                                                                                                                              |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `FC-ANTHROPIC-POST-001` | 请求包含 `model`、映射后的 `messages`、`max_tokens`、`stream`；`max_tokens` 取 `request.maxOutputTokens`，缺省 `defaultMaxTokens`。                                                                                                                                                               |
| `FC-ANTHROPIC-POST-002` | 所有 system 消息合并为顶层 `system` 字符串，按声明顺序以 `\n\n` 连接；空 system 不发送。                                                                                                                                                                                                          |
| `FC-ANTHROPIC-POST-003` | user 纯文本使用字符串；含图片时使用 `text` + `image` 内容块，图片为 `{ type: 'base64', media_type, data }`。                                                                                                                                                                                      |
| `FC-ANTHROPIC-POST-004` | assistant 的 `reasoning`：有 `redactedData` → `redacted_thinking` 块；有 `signature` → `thinking` 块（含文本与签名）；两者皆无则跳过；`text` → `text` 块，`tool_call` → `tool_use` 块；没有任何可映射块时抛错。                                                                                   |
| `FC-ANTHROPIC-POST-005` | tool 消息映射为 user 消息中的 `tool_result` 块（`tool_use_id`、`content`、可选 `is_error`）；连续 tool 消息合并进同一条 user 消息。                                                                                                                                                               |
| `FC-ANTHROPIC-POST-006` | `tools` → `{ name, description, input_schema }`；`toolChoice`：`auto`→`{type:'auto'}`、`none`→`{type:'none'}`、`required`→`{type:'any'}`、指定工具→`{type:'tool',name}`；没有工具时不发送 `tool_choice`。                                                                                         |
| `FC-ANTHROPIC-POST-007` | 非流式响应的 `text`/`thinking`/`redacted_thinking`/`tool_use` 按内容块顺序映射为 `text`/`reasoning`/`reasoning`/`tool_call`；thinking 的 `signature` 与 redacted `data` 保留在 reasoning 块；无内容块时生成 `text: ''`；仅推理无文本时补空文本块。                                                |
| `FC-ANTHROPIC-POST-008` | `stop_reason`：`end_turn`/`stop_sequence`→`end`（有工具调用时 `tool_calls`）、`max_tokens`→`max_tokens`、`tool_use`→`tool_calls`、`refusal`→`content_filter`、其他/缺省→`other`（有工具调用时 `tool_calls`）。                                                                                    |
| `FC-ANTHROPIC-POST-009` | usage 映射：`inputTokens = input_tokens + cache_creation_input_tokens + cache_read_input_tokens`；`outputTokens = output_tokens`；缓存字段分别映射 `cacheWriteTokens`/`cacheReadTokens`。                                                                                                         |
| `FC-ANTHROPIC-POST-010` | 流式按 `data:` 行解析，忽略 `event:`/空行/`ping`；支持 `message_start`、`content_block_start`、`content_block_delta`、`content_block_stop`、`message_delta`、`message_stop`、`error`；缺少 `message_stop` 的 EOF 也完成组装。                                                                     |
| `FC-ANTHROPIC-POST-011` | 流式增量：`text_delta`→`text_delta`、`thinking_delta`→`reasoning_delta`、`input_json_delta`→`tool_call_delta`；`signature_delta` 累积到当前 thinking 块但不单独转发；支持 `redacted_thinking` 起始块；工具起始增量携带 id/name 与空参数。最终 response 与非流式拥有相同消息/停止原因/usage 契约。 |
| `FC-ANTHROPIC-POST-012` | HTTP 非 2xx 抛 `AnthropicProviderError`，带 `status` 与截断到 1000 字符的响应体；错误体读取有字节上限；不包含 `x-api-key`。                                                                                                                                                                       |
| `FC-ANTHROPIC-POST-014` | `request.thinking` 映射：未配置时不添加字段；`isEnabled=false` → `{ type: 'disabled' }`；启用时 → `{ type: 'enabled', budget_tokens }`，预算取 `budgetTokens`，缺省 `defaultThinkingBudgetTokens`（默认 2048）。                                                                                  |
| `FC-ANTHROPIC-POST-013` | 非 JSON 响应、SSE JSON/error 非法、读取流失败、缺少 body、消息/usage/工具参数非法、网络请求失败均抛 `AnthropicProviderError` 并保留 `cause`（若有）。                                                                                                                                             |

## 5. 流状态机

```text
States  = { Connecting, Streaming, Assemble, Done }
Initial = Connecting
Final   = { Done }
```

| 当前状态     | 事件                     | 下一状态    | 副作用 / 拦截                      |
| ------------ | ------------------------ | ----------- | ---------------------------------- |
| `Connecting` | HTTP 2xx                 | `Streaming` | 获取 body reader                   |
| `Connecting` | HTTP 非 2xx              | —           | 抛 `AnthropicProviderError`        |
| `Connecting` | 无 body                  | —           | 抛 `AnthropicProviderError`        |
| `Streaming`  | `message_start`          | `Streaming` | 记录输入 usage                     |
| `Streaming`  | `content_block_start`    | `Streaming` | 建立内容块累积器并转发起始增量     |
| `Streaming`  | `content_block_delta`    | `Streaming` | 转发 text/reasoning/tool_call 增量 |
| `Streaming`  | `message_delta`          | `Streaming` | 更新停止原因与输出 usage           |
| `Streaming`  | `ping`                   | `Streaming` | 忽略                               |
| `Streaming`  | `message_stop` / EOF     | `Assemble`  | 停止读流                           |
| `Streaming`  | `error` / 非法 JSON/字段 | —           | 抛 `AnthropicProviderError`        |
| `Assemble`   | 组装成功                 | `Done`      | 发出唯一 `response` 事件           |

## 6. 错误分类

| 类型 / code                                           | 触发条件                                                     | 上下文                            |
| ----------------------------------------------------- | ------------------------------------------------------------ | --------------------------------- |
| `AnthropicProviderError` / `anthropic_provider_error` | 选项非法、HTTP 非 2xx、响应/流/usage/工具参数非法、body 缺失 | `providerId`、`status?`、`cause?` |
| `AgentModelError`（上游）                             | `@kurumi/agent` 捕获适配器错误后包装                         | `providerId`、`cause`             |

## 7. 边界采样

- URL：官方根地址、DeepSeek Anthropic 根地址、带尾斜杠、空字符串、非 http(s)、带 query/hash；
- 消息：system 多条、纯文本 user、文本+图片、reasoning+tool_call+text、带 signature/redactedData 的 reasoning、连续 tool 结果、非法角色、非法 tool 消息；
- 思考：`thinking` 缺省/true/false/预算对象、providerOptions 覆盖、thinking 签名与 redacted 数据、signature 非法类型、仅签名无文本；
- 工具：空参数、合法 JSON、非法 JSON、缺失 id/name、跨 chunk 参数、非空起始 input；
- usage：缺省、null、缓存字段、负数、小数、溢出；
- 流：`message_stop`、无 `message_stop`、`ping`、未知事件、空 content、非法 JSON、非 2xx、无 body、reader 错误；
- 安全：API Key 不出现在错误消息，headers 不能覆盖 `x-api-key`。

## 8. 追溯矩阵

| 约束                                                                                    | 测试文件 / 用例                                                                     |
| --------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| `PRE-001` / `PRE-002` / `PRE-004`                                                       | `src/provider.test.ts` · 选项默认值、自定义根地址、非法选项                         |
| `PRE-003`                                                                               | `src/provider.test.ts` · headers 合并与非法 headers                                 |
| `PRE-005` / `POST-001` / `POST-002` / `POST-003` / `POST-004` / `POST-005` / `POST-006` | `src/provider.test.ts` · 非流式请求映射（system/消息/图片/工具/工具结果/透传选项）  |
| `PRE-006` / `PRE-007` / `POST-007` / `POST-008` / `POST-009`                            | `src/provider.test.ts` · 响应映射、停止原因、缓存 usage、非法响应                   |
| `PRE-008`                                                                               | `src/provider.test.ts` · usage 非法计数、溢出                                       |
| `POST-012` / `I-ANTHROPIC-002`                                                          | `src/provider.test.ts`、`src/stream.test.ts` · HTTP 401 / SSE error 遮蔽 Key        |
| `PRE-009` / `POST-010` / `POST-011`                                                     | `src/stream.test.ts` · 增量转发、工具累积、缺失 message_stop、非法 delta、SSE error |
| `POST-013` 传输/读取错误                                                                | `src/provider.test.ts`、`src/stream.test.ts` · 网络错误、reader 错误包装            |
| `POST-013`                                                                              | `src/provider.test.ts`、`src/stream.test.ts` · 非 JSON/SSE JSON/无 body             |
| `I-ANTHROPIC-001`                                                                       | `src/architecture.test.ts` · 无 node/config/log/console 依赖                        |
| `I-ANTHROPIC-004`                                                                       | `src/provider.test.ts` · providerOptions 合并、核心字段不可覆盖                     |
| `I-ANTHROPIC-007`                                                                       | `src/stream.test.ts` · 最终 response 事件                                           |
| `I-ANTHROPIC-009`                                                                       | `src/provider.test.ts`、`src/stream.test.ts` · thinking → reasoning、块顺序         |
| `PRE-010` / `POST-014`                                                                  | `src/provider.test.ts` · thinking 请求映射、默认预算、关闭与 providerOptions 优先级 |
| `PRE-011` / `POST-007` / `POST-011` / `I-ANTHROPIC-009` / `I-ANTHROPIC-010`             | `src/provider.test.ts`、`src/stream.test.ts` · 签名/redacted 映射与回传             |

## 9. 兼容性与演进

- 破坏性变更：调整 `AnthropicProviderOptions` 必选字段、默认根地址/版本、`AnthropicProviderError` code、消息/工具映射语义、流事件顺序。
- 非破坏性变更：新增可选选项（必须提供默认值）、新增可选响应字段、扩展 usage 缓存映射、新增兼容的流事件类型。
- thinking 续传字段为可选：旧消息缺少 `signature`/`redactedData` 时继续按原语义跳过；新增 `defaultThinkingBudgetTokens` 必须提供默认值。
- 新增 provider 子包应复用 `@kurumi/agent` 的 `ModelProvider` 契约，不得把 vendor DTO 写入 Agent 核心。
- 修改公开 API 必须同步更新本契约、README、示例与测试。
