# @kurumi/agent 形式化契约

> 版本：0.1.0  
> 范围：`packages/agent/src`（消息、模型契约、工具、Agent 运行循环、事件、会话与 Token 用量）。工具输入 schema 复用 `@kurumi/schema`。  
> 判定：Agent SDK 是上层应用与模型/工具之间的对外契约类基础设施，涉及运行控制、并发、取消与不可信模型输入，**不适用** FSVDD §1.2 的纯文档/构建配置豁免。持久化契约采用本文件。

## 1. 范围与职责边界

`@kurumi/agent` 负责：

- 定义跨 runtime 的消息、内容块、模型提供方、工具与事件契约；
- 校验不可信模型响应与工具输入；
- 实现 Agent 循环：模型调用、工具执行、结果回传、轮数上限、重试、审批、取消；
- 提供进程内会话与不可变运行结果；
- 通过事件流暴露全部运行观测信息。

`@kurumi/agent` 不负责：

- HTTP/SDK 模型适配器（由 provider 包或应用实现 `ModelProvider`）；
- 会话持久化、分布式锁与多实例一致性（由调用方实现）；
- 日志、指标或链路追踪（**不依赖 `@kurumi/log`，不写 console**；观测统一走事件流）；
- 配置加载（应用可把 `@kurumi/config` 的产物映射为 `AgentConfig`）；
- 模型侧 token 预算裁剪、RAG、MCP、子 Agent 编排等上层能力。

## 2. 类型与不变量

### 2.1 核心类型

```text
AgentMessageRole = system | user | assistant | tool
AgentContentPart = Text | Image | Reasoning | ToolCall | ToolResult
Reasoning        = { type: "reasoning", text, signature?, redactedData? }
AgentMessage     = { role, content: AgentContentPart[] }
ToolDescriptor   = { name, description, inputSchema: JsonValue }
ToolDefinition   = { name, description, inputSchema: Schema<TInput> | JsonValue, parseInput?, execute, timeoutMs? }
ThinkingConfig   = boolean | { isEnabled?, budgetTokens?, effort? }
ModelProvider    = { id, generate(), stream?() }
ModelRequest     = { model, messages, tools, toolChoice?, temperature?, maxOutputTokens?, thinking?, providerOptions? }
ModelResponse    = { message: AgentMessage, stopReason: ModelStopReason, usage?: TokenUsage }
AgentConfig      = { provider, model, system?, tools?, maxSteps?, thinking?, ... }
AgentResult      = { messages, generatedMessages, finalMessage, steps, usage, stopReason, modelStopReason, retryCount }
TokenUsage       = { inputTokens, outputTokens, totalTokens, cacheReadTokens?, cacheWriteTokens? }
```

### 2.2 系统不变量

| 编号          | 不变量                                                                                                                                     |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `I-AGENT-001` | 主入口及其非测试源码不得导入 `node:`、`@kurumi/log`、`@kurumi/config`，也不得直接调用 `console.*`；工具 schema 统一来自 `@kurumi/schema`。 |
| `I-AGENT-002` | 不修改调用方传入的配置、消息、工具定义或模型请求；消息、事件、结果、schema 与工具输入递归冻结。                                            |
| `I-AGENT-003` | 工具输入、工具元数据、schema 与透传选项必须是 JSON 安全值；危险键、函数、Date、稀疏数组等必须显式拒绝。                                    |
| `I-AGENT-004` | 单次运行最多发起 `maxSteps` 次模型调用；每次成功响应恰好生成一条 assistant 消息。                                                          |
| `I-AGENT-005` | 契约/模型/工具实现错误必须带稳定 `code` 并保留 `cause`；取消与轮数上限是正常结果，不得抛错。                                               |
| `I-AGENT-006` | 工具不存在、解析失败、执行抛错或审批拒绝必须转换为 `isError=true` 的工具结果回传模型，不得中断运行。                                       |
| `I-AGENT-007` | 每条含工具调用的 assistant 消息后，必须按调用顺序追加数量相等、callId 对应的 tool 消息；取消时也必须补齐取消结果。                         |
| `I-AGENT-008` | 重复 callId 属于模型协议错误，必须在执行任何工具前拒绝；未知工具不执行，只产生错误结果。                                                   |
| `I-AGENT-009` | 工具并发数不得超过 `toolConcurrency`；工具结束事件按实际完成顺序发出，回传消息按调用顺序追加。                                             |
| `I-AGENT-010` | Agent、注册表与会话不持有全局可变状态；同一实例可被多次调用。                                                                              |
| `I-AGENT-011` | `thinking` 缺省或布尔或对象；对象中 `isEnabled` 必须是布尔，`budgetTokens` 必须是正安全整数，`effort` 必须属于受限枚举；归一化后递归冻结。 |
| `I-AGENT-012` | `reasoning` 的 `signature`/`redactedData` 是提供方不透明续传数据：二者互斥，Agent 不解析内容，只原样保存与回传。                           |

## 3. 前置约束

| 编号               | 约束                                                                                                                                                                                                                   | 失败语义                                         |
| ------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| `FC-AGENT-PRE-001` | `config` 必须是对象；`provider` 必须有非空 `id` 与 `generate` 函数；可选 `stream` 必须是函数。                                                                                                                         | `AgentDefinitionError`                           |
| `FC-AGENT-PRE-002` | `model` 必须非空；`system` 若提供必须非空。                                                                                                                                                                            | `AgentDefinitionError`                           |
| `FC-AGENT-PRE-003` | `maxSteps`、`toolConcurrency`、`maxToolResultLength` 必须是正安全整数；`toolTimeoutMs`、`maxOutputTokens` 若提供必须是正安全整数；`maxModelRetries`、`retryDelayMs` 必须是非负安全整数；`temperature` 必须是有限数值。 | `AgentDefinitionError`                           |
| `FC-AGENT-PRE-004` | 工具名称必须匹配 `^[A-Za-z][A-Za-z0-9_-]{0,63}$`，描述非空，`execute` 为函数，`parseInput`/`timeoutMs` 若提供必须合法；`inputSchema` 必须是 `Schema` 实例或 JSON Schema 对象，名称不得重复。                           | `AgentDefinitionError`                           |
| `FC-AGENT-PRE-011` | 传入 `Schema<TInput>` 时，`createTool` 必须导出 JSON Schema，并在未提供 `parseInput` 时默认使用 `parseOrThrow` 校验输入；显式 `parseInput` 优先。                                                                      | `AgentDefinitionError` / `SchemaValidationError` |
| `FC-AGENT-PRE-005` | `messages` 必须是消息数组；逐条归一化；注入 system 后至少一条消息。若首条已是 system，则不再注入配置的 `system`。                                                                                                      | `AgentDefinitionError`                           |
| `FC-AGENT-PRE-006` | 角色/内容矩阵：system 仅 text；user 仅 text/image；assistant 仅 text/reasoning/tool_call；tool 恰好一个 tool_result。工具名、callId 非空且同一消息内 callId 唯一；输入/元数据 JSON 安全。                              | `AgentMessageError`                              |
| `FC-AGENT-PRE-007` | 运行选项必须是对象；`signal` 若提供必须具备 `aborted`、`addEventListener`、`removeEventListener`；`maxSteps`/`toolChoice`/`shouldStream` 覆盖值必须合法。                                                              | `AgentDefinitionError`                           |
| `FC-AGENT-PRE-008` | `toolChoice` 只能是 `auto` / `none` / `required` / `{ type: "tool", name }`；指定工具必须已注册；`required` 需要至少一个工具。                                                                                         | `AgentDefinitionError`                           |
| `FC-AGENT-PRE-009` | 模型响应必须是普通对象，消息必须是合法 assistant 消息，`stopReason` 必须合法，`usage` 若提供必须满足 TokenUsage；流事件字段必须合法且最终必须出现 response。                                                           | `AgentModelError`                                |
| `FC-AGENT-PRE-010` | `approveToolCall` 与 `shouldRetryModelError` 若提供必须是函数；审批必须返回布尔值。                                                                                                                                    | `AgentDefinitionError` / `AgentToolError`        |
| `FC-AGENT-PRE-012` | `thinking` 若提供必须是布尔或普通对象；`isEnabled` 必须为布尔；`budgetTokens` 必须是正安全整数；`effort` 只能为 `minimal`/`low`/`medium`/`high`。运行选项中的 `thinking` 覆盖构造配置。                                | `AgentDefinitionError`                           |
| `FC-AGENT-PRE-013` | `reasoning` 内容块的 `signature`/`redactedData` 若提供必须是非空字符串，且不得同时存在。                                                                                                                               | `AgentMessageError`                              |

## 4. 后置约束

| 编号                | 约束                                                                                                                                                                                |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `FC-AGENT-POST-001` | 返回的消息、事件、结果与会话快照全部冻结；调用方输入不被修改。                                                                                                                      |
| `FC-AGENT-POST-002` | 模型请求包含冻结的消息快照、工具描述、工具选择、温度、最大输出 Token 与冻结的 `providerOptions`；不修改注册表。                                                                     |
| `FC-AGENT-POST-003` | 每次成功模型响应被追加为一条 assistant 消息；`steps` 等于成功响应数；`usage` 为各轮用量之和；`modelStopReason` 为最后一次响应的停止原因。                                           |
| `FC-AGENT-POST-004` | 工具调用按 `toolConcurrency` 并发执行；`tool_start` 先于对应 `tool_end`；结果按调用顺序追加为 tool 消息。                                                                           |
| `FC-AGENT-POST-005` | 未知工具、输入解析抛错、执行抛错、超时、审批拒绝均转为 `isError=true` 结果；工具返回非法结构抛 `AgentToolError`。                                                                   |
| `FC-AGENT-POST-006` | 取消信号触发后不再发起新的模型调用；模型调用/工具执行/审批/重试等待中的取消返回 `stopReason=aborted`，不抛错；已开始的工具调用补齐“工具调用已取消”结果。                            |
| `FC-AGENT-POST-007` | 达到 `maxSteps` 时，最后一轮仍执行工具并追加结果，然后以 `stopReason=max_steps` 结束；`finalMessage` 仅在正常完成时存在。                                                           |
| `FC-AGENT-POST-008` | 模型调用失败且未取消时，按 `maxModelRetries` 与 `shouldRetryModelError` 重试；每次重试发出 `model_retry` 事件，`retryCount` 为累计重试次数；等待期间可取消。                        |
| `FC-AGENT-POST-009` | 提供方实现 `stream` 且 `shouldStream=true` 时优先流式；每个流增量转发为 `model_delta`，最终 response 组装为 assistant 消息；流缺少 response 或事件非法时抛模型错误。                |
| `FC-AGENT-POST-010` | 工具结果超过 `maxToolResultLength` 时截断，截断标记计入长度预算，最终 `content.length <= maxToolResultLength`；错误结果同样受限。                                                   |
| `FC-AGENT-POST-011` | `runInSession` 只把 `generatedMessages` 追加到会话；运行抛错时会话保持不变；system 不写入会话，下一次运行重新注入。                                                                 |
| `FC-AGENT-POST-012` | 事件顺序：`agent_start` → 每轮 `step_start` →（`model_delta`/`model_retry`）→ `assistant_message` →（`tool_start`/`tool_end`）→ `step_end` → `agent_end`。取消可省略未发生的阶段。  |
| `FC-AGENT-POST-013` | 正常完成且无工具调用时 `stopReason=completed`、`finalMessage` 为最后 assistant 消息；内容过滤/最大 Token 等其他模型停止原因不改变 Agent 级 `completed`，由 `modelStopReason` 区分。 |
| `FC-AGENT-POST-014` | 工具级 `timeoutMs` 优先于 Agent 级 `toolTimeoutMs`；超时工具收到 abort 信号，结果文本包含超时毫秒数。                                                                               |
| `FC-AGENT-POST-015` | `createTool` 返回的 `inputSchema` 始终是冻结的 JSON Schema；传入 schema 时由 `toJsonSchema()` 生成，required/additionalProperties/default 等映射遵循 `@kurumi/schema` 契约。        |
| `FC-AGENT-POST-016` | 模型请求仅在 `thinking` 已配置时携带归一化、冻结的 `thinking` 快照；缺省不添加该字段；单次运行覆盖值优先于构造配置。                                                                |
| `FC-AGENT-POST-017` | `assistant_message` 事件、`generatedMessages` 与运行结果保留 `reasoning` 的 `signature`/`redactedData`；`extractMessageReasoning` 按内容块顺序拼接推理文本，不包含签名数据。        |

## 5. 运行状态机

```text
M = (States, Events, Transition_Function, Initial_State, Final_States)
States  = { ModelCall, ToolExecution, Completed, MaxSteps, Aborted }
Events  = { ModelResponse, ToolCalls, NoToolCalls, ToolsDone, Limit, Abort, Failure }
Initial = ModelCall
Final   = { Completed, MaxSteps, Aborted }
```

| 当前状态        | 事件                | 下一状态        | 副作用 / 拦截                                      |
| --------------- | ------------------- | --------------- | -------------------------------------------------- |
| `ModelCall`     | `NoToolCalls`       | `Completed`     | 追加 assistant，发出 step_end / agent_end          |
| `ModelCall`     | `ToolCalls`         | `ToolExecution` | 追加 assistant，发出 tool_start                    |
| `ModelCall`     | `Abort`             | `Aborted`       | 不追加半截消息，发出 agent_end                     |
| `ModelCall`     | `Failure`           | `ModelCall`     | 未达重试上限时等待后重试；否则抛 `AgentModelError` |
| `ToolExecution` | `ToolsDone`         | `ModelCall`     | 按调用顺序追加 tool 消息                           |
| `ToolExecution` | `ToolsDone + Limit` | `MaxSteps`      | 追加 tool 消息后停止                               |
| `ToolExecution` | `Abort`             | `Aborted`       | 追加取消结果后停止                                 |
| `ToolExecution` | `Failure`           | —               | 工具返回值非法/调度不变量破坏时抛错                |

非法转移：

- 模型返回 `stopReason=tool_calls` 但消息没有工具调用 → `AgentModelError`；
- 同一响应重复 callId → `AgentModelError`，且不执行任何工具；
- 工具返回非字符串/非 `ToolResultInput` → `AgentToolError`。

## 6. 错误分类

| 类型 / code                                       | 触发条件                                                      | 关键上下文            |
| ------------------------------------------------- | ------------------------------------------------------------- | --------------------- |
| `AgentDefinitionError` / `agent_definition_error` | 配置、工具定义、运行选项、toolChoice、透传选项非法            | 直接消息、`cause`     |
| `AgentMessageError` / `agent_message_error`       | 消息结构/角色矩阵/JSON 输入非法                               | `path`、`cause`       |
| `AgentModelError` / `agent_model_error`           | 适配器调用失败、响应/流事件/usage/stopReason 非法、协议不一致 | `providerId`、`cause` |
| `AgentToolError` / `agent_tool_error`             | 工具返回值非法、审批回调失败或返回非布尔、内部调度失败        | `toolName`、`cause`   |
| `AgentResult.stopReason=aborted`                  | 调用方取消；不是异常                                          | 取消结果              |
| `AgentResult.stopReason=max_steps`                | 达到轮数上限；不是异常                                        | 已追加工具结果        |

## 7. 边界与采样

- 数值：`0`、`1`、上界、`Bound+1`、负数、小数、`NaN`、`±Infinity`、`Number.MAX_SAFE_INTEGER`。
- 消息：空数组、纯空白、Unicode/Emoji、超长文本、图片非法 MIME/空数据、`__proto__` 输入、缺省 `input`、重复 callId。
- 思考：`thinking` 缺省、布尔、空对象、非法 `budgetTokens`（0/负数/小数/溢出）、非法 `effort`、运行期覆盖；reasoning 带/不带 `signature`、带 `redactedData`、两者同时出现。
- 工具：未知名称、解析抛错、执行抛错、非法返回、超时、取消、审批拒绝/抛错/非布尔、超过结果长度上限。
- 模型：非对象响应、user 角色响应、非法 stopReason、负数 usage、流缺 response、未知流事件、重复/非法增量。
- 并发/取消：审批中取消、工具中取消、流式中取消、重试等待中取消、启动前已取消、最后一轮工具调用。

## 8. 追溯矩阵

| 约束                                                      | 测试文件 / 用例                                                                                |
| --------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| `PRE-001` / `PRE-002` / `PRE-003` / `PRE-007` / `PRE-010` | `agent-config.test.ts` · 配置校验、运行选项覆盖                                                |
| `PRE-004` / `PRE-011` / `POST-015`                        | `tool.test.ts` · createTool / createToolRegistry 边界、schema 导出、自动 parseInput、类型推导  |
| `PRE-005` / `PRE-006`                                     | `message.test.ts`、`agent-run.test.ts` · 角色矩阵、system 注入                                 |
| `PRE-008`                                                 | `model.test.ts`、`agent-advanced.test.ts` · toolChoice                                         |
| `PRE-009`                                                 | `model.test.ts`、`agent-run.test.ts` · 响应与流事件校验                                        |
| `POST-001` / `I-AGENT-002`                                | `message.test.ts`、`agent-run.test.ts`、`properties.test.ts` · 冻结与克隆                      |
| `POST-002`                                                | `agent-run.test.ts` · 请求快照与 providerOptions                                               |
| `POST-003` / `POST-013`                                   | `agent-run.test.ts` · 步骤、usage、停止原因                                                    |
| `POST-004` / `I-AGENT-009`                                | `agent-advanced.test.ts` · 并发上限与结果顺序                                                  |
| `POST-005` / `I-AGENT-006`                                | `agent-run.test.ts` · 未知工具、执行/解析错误、非法返回                                        |
| schema 依赖                                               | `architecture.test.ts` · 工具 schema 统一来自 `@kurumi/schema`                                 |
| `POST-006` / `POST-014` / `I-AGENT-005`                   | `agent-run.test.ts`、`agent-advanced.test.ts` · 启动前/模型中/工具中/流式/重试取消、超时       |
| `POST-007`                                                | `agent-run.test.ts`、`agent-advanced.test.ts` · maxSteps                                       |
| `POST-008`                                                | `agent-advanced.test.ts` · 重试、判定回调、等待取消                                            |
| `POST-009`                                                | `agent-advanced.test.ts` · 流式转发、fallback、缺 response、取消                               |
| `POST-010`                                                | `agent-advanced.test.ts` · 结果截断且最终长度不超过上限                                        |
| `PRE-006` callId 唯一                                     | `message.test.ts` · 同一消息内重复 callId 被拒绝                                               |
| `POST-011`                                                | `session.test.ts`、`agent-config.test.ts` · 会话追加与失败原子性                               |
| `POST-012`                                                | `agent-run.test.ts`、`agent-advanced.test.ts` · 事件顺序                                       |
| `I-AGENT-001`                                             | `architecture.test.ts` · 无 node/config/log/console 依赖                                       |
| `I-AGENT-004` / `I-AGENT-007` / `I-AGENT-008`             | `agent-run.test.ts` · 轮数、工具消息配对、重复 callId                                          |
| `PBT-AGENT-001`                                           | `properties.test.ts` · 任意文本与 JSON 工具输入                                                |
| `PRE-012` / `POST-016`                                    | `model.test.ts`、`agent-config.test.ts`、`agent-run.test.ts` · thinking 归一化、覆盖与请求快照 |
| `PRE-013` / `POST-017`                                    | `message.test.ts`、`agent-run.test.ts` · reasoning 签名/加密数据保存与提取                     |

## 9. 兼容性与演进

- 破坏性变更：调整 `AgentMessage` 角色/内容块判别字段、`ModelProvider` 方法签名、`ModelResponse` 必选字段、`AgentResult` 必选字段、`ToolDefinition.execute` 签名、默认 `maxSteps`/并发/截断上限、事件 `type` 名称。
- 非破坏性变更：新增可选配置字段（必须提供缺省值）、新增可选内容块、新增可选模型流事件、新增事件类型、放宽 JSON 输入但不降低安全性、`ToolDefinition.inputSchema` 接受 `Schema` 实例。
- `thinking` 属于可选请求配置：缺省时适配器不得添加思考字段；`ReasoningContentPart.signature`/`redactedData` 为可选续传字段，缺失时旧消息仍可正常使用。
- `inputSchema` 允许 schema 后，`createTool` 的返回类型为 `NormalizedToolDefinition`（`inputSchema` 已是 JSON Schema）；原 JSON Schema + `parseInput` 用法保持兼容。
- 新增 provider 子包时必须通过 `ModelProvider` 契约，不得把 vendor 字段写入核心消息；vendor 选项走 `providerOptions`。
- 修改公开 API（参数/返回值/抛错/事件）必须同步更新本契约、TSDoc、README、示例与测试，并在提交信息标注 `!`。
- 契约与代码不一致时以本文件为唯一真实源，先更新契约再改实现。
