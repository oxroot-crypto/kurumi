---
'@kurumi/core': minor
'@kurumi/schema': minor
'@kurumi/config': minor
'@kurumi/log': minor
'@kurumi/agent': minor
'@kurumi/open-ai': minor
'@kurumi/anthropic': minor
---

# Kurumi 初始版本

首个可发布版本，建立跨 runtime 的 Agent SDK 基础设施：从底层错误、JSON 值、schema、配置与日志，到 Agent 运行循环及 OpenAI / Anthropic 协议适配器。所有包均以 ESM 发布，主入口尽量避免 Node 依赖，并提供显式的 `node` 子路径。

## `@kurumi/core`

- 新增统一错误基类 `KurumiError`，提供稳定 `code`、`cause` 因果链与正确的子类名。
- 新增 JSON 值工具：`isPlainObject`、`isForbiddenKey`、`cloneJsonValue`、`deepFreeze`、`isDeepEqual`，包含原型污染防护、稀疏数组/非有限数值拒绝与 64 层深度上限。
- 新增 JSON 安全序列化：`toJsonSafeValue`、`serializeError`，覆盖循环引用、深度/长度截断、`AggregateError`、Map/Set/Date/Symbol/BigInt 的确定性表示。
- 新增 `RuntimeHost` 抽象与 `createMemoryRuntimeHost`、`createNodeRuntimeHost` 实现，支持取消信号与 `FileNotFoundError` / `RuntimeHostError` 语义。
- 新增跨 realm 的 `isAbortSignal` / `isAbortError` 判定。
- 主入口不导入 `node:`；Node 能力仅通过 `@kurumi/core/node` 暴露。

## `@kurumi/schema`

- 新增零运行时依赖的 schema 系统：`object`、`record`、`array`、`union`、`string`、`number`、`boolean`、`literal`、`oneOf`。
- 支持 `optional` / `nullable` / `default` 包装、类型推导、运行时校验与结构化路径错误。
- 新增 `SchemaDefinitionError` / `SchemaValidationError`，失败时携带稳定 `code` 与字段路径。
- 新增敏感值遮蔽与 `toJsonSchema()` 导出，供配置与工具定义复用。
- 默认严格遵循 JSON 语义；仅显式开启 `shouldCoerceStrings` 时执行字符串强转。
- 默认拒绝原型污染键、限制嵌套深度、克隆默认值并支持递归冻结。

## `@kurumi/config`

- 新增配置加载链路：`loadConfig`、多层 `sources` 合并、`fileSource` / `envSource` / `objectSource`。
- 新增 `createConfigStore` 可重载 store，支持订阅、重载与不可变快照。
- 新增环境变量前缀映射、嵌套 section 映射与 `${VAR}` 字符串插值；缺失变量显式报错。
- 复用 `@kurumi/schema` 做类型安全校验，聚合多个字段错误并携带路径。
- Node 文件与环境变量能力通过 `@kurumi/config/node` 注入；核心入口保持 runtime 无关。

## `@kurumi/log`

- 新增分级日志器 `createLogger`：`trace` / `debug` / `info` / `warn` / `error` / `fatal` / `silent`。
- 新增结构化字段、`child()` 绑定、错误链序列化与 JSON 安全归一化。
- 新增可插拔 sink：`createConsoleSink`、`createMemorySink`，以及 `@kurumi/log/node` 下的 `createNodeFileSink`。
- 同步 sink 失败直接抛出；异步 sink 失败由 `onSinkError` 或 `flush()` / `close()` 显式聚合，不静默吞错。
- 交给 sink 的记录递归冻结，保证多个 sink 观察到同一份不可变数据。

## `@kurumi/agent`

- 新增消息契约：文本、图片、推理、工具调用与工具结果内容块，支持 `system` / `user` / `assistant` / `tool` 角色矩阵校验。
- 新增模型抽象 `ModelProvider` 与 `ModelRequest` / `ModelResponse`，适配同步生成与流式生成。
- 新增 `createTool` 与工具注册表：schema 输入校验、JSON Schema 导出、并发执行、超时、审批与结果截断。
- 新增完整 Agent 循环：轮数上限、模型重试、取消信号、工具错误回传、`max_steps` / `aborted` 正常终止语义。
- 新增事件流：`agent_start`、`step_start`、`model_delta`、`model_retry`、`assistant_message`、`tool_start`、`tool_end`、`step_end`、`agent_end`。
- 新增进程内会话 `createAgentSession` / `runInSession` 与不可变 `AgentResult`。
- 模型响应、工具输入、配置与结果全部在边界校验并递归冻结；错误带稳定 `code`、上下文字段与 `cause`。
- 支持 `thinking` 配置与 `reasoning` 不透明续传数据（`signature` / `redactedData`），主入口不依赖 Node、`@kurumi/log` 或 `@kurumi/config`。

## `@kurumi/open-ai`

- 新增 OpenAI Chat Completions 兼容 `ModelProvider`。
- 支持消息、工具调用、`tool_choice`、`finish_reason` 与 usage 映射。
- 支持 SSE 流式文本、推理字段与工具参数增量，并可选包含流式 usage。
- 支持 `providerOptions` 透传、可配置 `maxTokensField` / `reasoningContentField` / `thinkingField`。
- 错误携带 `providerId` 与 HTTP `status`；使用全局 `fetch` 且支持注入自定义 `fetch`。

## `@kurumi/anthropic`

- 新增 Anthropic Messages 协议兼容 `ModelProvider`。
- 支持 system / user / assistant 消息、图片、工具、`tool_choice`、`stop_reason` 与 usage 映射。
- 连续工具结果自动合并为单条 user 消息，符合 Anthropic `tool_result` 结构。
- 支持 SSE 流式文本、thinking 与 `input_json_delta` 工具参数增量。
- 支持扩展思考预算、`anthropic-version`、默认 `max_tokens`、自定义 headers 与 `fetch` 注入。
- 错误携带 `providerId` 与 HTTP `status`；主入口零 Node 依赖。

## 工程与文档

- 新增 pnpm workspace、TypeScript strict 配置、oxlint、Prettier、Vitest 与 fast-check 属性测试。
- 新增 Changesets 发布流程、GitHub Actions CI（lint、format、typecheck、coverage、build）。
- 每个包均提供 README；在 `docs/spec/` 下为全部模块补充 FSVDD 形式化契约、约束矩阵与追溯矩阵。
- 新增 `examples/deepseek-agent` 真实环境验证示例，覆盖 OpenAI 与 Anthropic 两种协议。
