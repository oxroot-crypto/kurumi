# @kurumi/agent

Kurumi 的跨 runtime Agent SDK 基础设施：**消息契约 + 模型抽象 + 工具执行 + Agent 循环 + 事件流 + 会话**。

- **跨 runtime**：主入口不依赖 `node:`、`@kurumi/config` 或 `@kurumi/log`。
- **契约优先**：模型响应、工具输入与配置全部在边界处校验并递归冻结。
- **完整运行控制**：轮数上限、工具并发、审批、超时、结果截断、模型重试、取消信号。
- **流式优先**：提供方实现 `stream` 时自动转发文本/推理/工具参数增量。
- **错误可见**：工具失败转为模型可见结果；契约错误带稳定 `code`、上下文字段与 `cause`。
- **无内置日志**：不写 console、不硬编码日志；观察者通过事件流或自行注入 logger。
- **会话与结果可持久化**：消息和结果都是 JSON 安全、递归冻结的数据。

## 安装与导入

```ts
import {
  AgentMessageRole,
  AgentStopReason,
  ModelStopReason,
  createAgent,
  createAgentSession,
  createTool,
  createUserMessage,
  extractMessageReasoning,
} from '@kurumi/agent';
```

## 快速开始

```ts
import { createAgent, createUserMessage } from '@kurumi/agent';
import { createOpenAIProvider } from '@kurumi/open-ai';

const apiKey = process.env.KURUMI_API_KEY;
if (apiKey === undefined || apiKey.trim() === '') throw new Error('KURUMI_API_KEY is required');

const agent = createAgent({
  provider: createOpenAIProvider({
    id: 'deepseek',
    apiKey,
    baseUrl: 'https://api.deepseek.com',
  }),
  model: 'deepseek-chat',
  system: '你是简洁的中文助手。',
});

const result = await agent.run([createUserMessage('用一句话解释 Agent SDK')]);
console.log(result.finalMessage);
```

`AgentConfig` 默认值：

| 选项                  | 默认值   | 说明                         |
| --------------------- | -------- | ---------------------------- |
| `maxSteps`            | `8`      | 最大模型轮数                 |
| `toolConcurrency`     | `1`      | 单轮工具最大并发             |
| `toolTimeoutMs`       | 不超时   | 工具默认超时                 |
| `maxToolResultLength` | `100000` | 单条工具结果字符上限         |
| `shouldStream`        | `true`   | 提供方有 `stream` 时优先流式 |
| `thinking`            | 不发送   | 思考配置，详见下文           |
| `maxModelRetries`     | `0`      | 模型调用失败重试次数         |
| `retryDelayMs`        | `500`    | 重试等待时间                 |

## 消息

```ts
import {
  createAssistantMessage,
  createSystemMessage,
  createToolMessage,
  createUserMessage,
} from '@kurumi/agent';

createSystemMessage('系统规则');
createUserMessage('文本');
createUserMessage([
  { type: 'text', text: '看图' },
  { type: 'image', mediaType: 'image/png', data: '...base64...' },
]);
createAssistantMessage([
  { type: 'reasoning', text: '思考过程' },
  { type: 'tool_call', callId: 'c1', name: 'search', input: { query: 'kurumi' } },
  { type: 'text', text: '回答' },
]);
createToolMessage({ callId: 'c1', name: 'search', content: '结果' });
```

角色/内容矩阵：

| 角色        | 允许的内容块                     |
| ----------- | -------------------------------- |
| `system`    | `text`                           |
| `user`      | `text`、`image`                  |
| `assistant` | `text`、`reasoning`、`tool_call` |
| `tool`      | 恰好一个 `tool_result`           |

工具调用输入必须是 JSON 安全值；危险键、函数、Date、稀疏数组会被拒绝。消息、内容块与输入全部递归冻结。

## 模型提供方

```ts
interface ModelProvider {
  readonly id: string;
  generate(request: ModelRequest, options: ModelCallOptions): Promise<ModelResponse>;
  stream?(request: ModelRequest, options: ModelCallOptions): AsyncIterable<ModelStreamEvent>;
}
```

- `generate` 必选；`stream` 可选。提供 `stream` 时，Agent 默认优先使用。
- 流事件包括 `text_delta`、`reasoning_delta`、`tool_call_delta`、`response`。
- 流必须以且仅以一个 `response` 事件结束；Agent 用它组装最终 assistant 消息。
- 请求中的 `messages` 与 `tools` 已冻结，适配器不得修改。
- vendor 专属字段通过 `providerOptions` 透传（JSON 安全值）。
- 思考配置通过 `ModelRequest.thinking` 传递；是否支持以及如何映射由提供方决定。

## 思考（Thinking）

```ts
const agent = createAgent({
  provider,
  model: 'deepseek-flash',
  thinking: {
    isEnabled: true,
    budgetTokens: 2048, // 预算型提供方使用
    effort: 'high', // effort 型提供方使用
  },
});

for await (const event of agent.stream([createUserMessage('解释一下 Agent SDK')])) {
  if (event.type === 'model_delta' && event.delta.type === 'reasoning_delta') {
    process.stderr.write(event.delta.text);
  }
}

const result = await agent.run([createUserMessage('你好')]);
const reasoning =
  result.finalMessage === undefined ? '' : extractMessageReasoning(result.finalMessage);
```

- `thinking` 可传布尔值或对象；`false` 表示显式关闭，缺省时不向提供方添加任何思考字段。
- 单次运行可用 `agent.stream(messages, { thinking: true })` 覆盖构造配置。
- `reasoning` 内容块可携带提供方不透明字段 `signature` / `redactedData`；Agent 原样保存，供 Anthropic 等适配器在工具轮次回传。
- `extractMessageText` 不包含推理文本；需要时使用 `extractMessageReasoning`。

```ts
const provider: ModelProvider = {
  id: 'my-provider',
  async generate(request, { signal }) {
    const response = await callMyApi(request, signal);
    return {
      message: createAssistantMessage(response.text),
      stopReason: ModelStopReason.End,
      usage: createTokenUsage(response.inputTokens, response.outputTokens),
    };
  },
  async *stream(request, { signal }) {
    for await (const chunk of callMyApiStream(request, signal)) {
      if (chunk.text) yield { type: 'text_delta', text: chunk.text };
    }
    yield { type: 'response', response: await finishResponse(request) };
  },
};
```

## 工具

```ts
import { createTool } from '@kurumi/agent';
import { number, object, string } from '@kurumi/schema';

const search = createTool({
  name: 'search',
  description: '搜索文档',
  inputSchema: object({
    query: string({ isNonEmpty: true }).describe('搜索词'),
    limit: number({ isInteger: true, min: 1, max: 20 }).default(5),
  }),
  timeoutMs: 10_000,
  async execute({ query, limit }, { callId, signal, messages }) {
    const result = await searchDocs(query, limit, { signal });
    return {
      content: result.summary,
      metadata: { callId, count: result.count },
    };
  },
});
```

- `inputSchema` 既可以是 `@kurumi/schema` 的强类型 schema，也可以是原始 JSON Schema（此时建议提供 `parseInput` 自行校验）。
- 传入 schema 时，`createTool` 自动导出 JSON Schema，并默认用 `parseOrThrow` 校验输入；显式 `parseInput` 优先。
- `parseInput`、`execute` 抛错 → `isError=true` 结果回传模型，运行继续。
- 未知工具、审批拒绝、超时、取消同样转为错误结果。
- 工具返回非字符串/非 `ToolResultInput` → 抛 `AgentToolError`（实现契约错误）。
- `timeoutMs` 优先于 Agent 级 `toolTimeoutMs`；超时会让工具收到 abort 信号。
- 结果超过 `maxToolResultLength` 时截断；截断标记计入长度预算，最终结果不超过该上限。

审批：

```ts
createAgent({
  provider,
  model: 'deepseek-chat',
  tools: [search],
  approveToolCall: async (call, { messages, signal }) => {
    return await ui.confirm(`允许调用 ${call.name}？`, { messages, signal });
  },
});
```

## 运行与事件

```ts
const stream = agent.stream([createUserMessage('现在几点？')], {
  signal: AbortSignal.timeout(60_000),
  maxSteps: 4,
});

let next = await stream.next();
while (!next.done) {
  const event = next.value;
  if (event.type === 'model_delta' && event.delta.type === 'text_delta') {
    process.stdout.write(event.delta.text);
  }
  next = await stream.next();
}
const result = next.value;
```

事件顺序：

```text
agent_start
  step_start
    model_delta* / model_retry*
    assistant_message
    tool_start / tool_end*
    step_end
agent_end
```

| 事件                | 说明                             |
| ------------------- | -------------------------------- |
| `agent_start`       | 注入 system 后的初始消息快照     |
| `step_start`        | 从 1 开始的模型轮次              |
| `model_delta`       | 文本/推理/工具参数增量           |
| `model_retry`       | 重试次数、等待时间与序列化错误   |
| `assistant_message` | 已校验消息、停止原因与本轮 usage |
| `tool_start`        | callId、工具名与输入快照         |
| `tool_end`          | 归一化结果（含 `isError`）       |
| `step_end`          | 本轮模型停止原因                 |
| `agent_end`         | 最终 `AgentResult`               |

停止原因：

- `completed`：模型给出最终 assistant 消息；
- `max_steps`：达到轮数上限（最后一轮工具仍会执行并回传）；
- `aborted`：调用方取消（**正常结果，不抛错**）。

`AgentResult` 同时包含 `modelStopReason`（`end` / `max_tokens` / `tool_calls` / `content_filter` / `other`）与累计 `usage`、`retryCount`。

> `result.messages` 会包含配置的 `system` 消息；把完整结果再次传给 `agent.run()` 时，首条 system 不会被重复注入。

## 重试

```ts
const agent = createAgent({
  provider,
  model: 'deepseek-chat',
  maxModelRetries: 2,
  retryDelayMs: 500,
  shouldRetryModelError: (error, attempt) => attempt <= 2,
});
```

- 只重试模型调用失败（包括响应校验失败），不重试工具错误。
- 每次重试发出 `model_retry`，错误已用 `serializeError` 转为 JSON 安全结构。
- 等待期间取消立即返回 `aborted`，`retryCount` 保留已发生次数。

## 会话

```ts
const session = createAgentSession({ messages: [createUserMessage('你好')] });

await agent.runInSession(session);
await agent.runInSession(session);
```

- 会话只保存对话消息；配置的 `system` 不入库，每次运行重新注入。
- `runInSession` 只追加本次 `generatedMessages`；运行抛错时会话保持不变。
- `messages` 每次读取都是冻结快照；会话不提供并发保护，同一会话并行运行需调用方加锁。
- 无配置 `system` 时，空会话不能启动运行。

## 错误处理

```ts
import { AgentError, AgentModelError } from '@kurumi/agent';

try {
  await agent.run(messages);
} catch (error) {
  if (error instanceof AgentModelError) {
    report(error.providerId, error.cause);
  } else if (error instanceof AgentError) {
    report(error.code, error.message);
  }
}
```

| 类型                   | code                     | 场景                                       |
| ---------------------- | ------------------------ | ------------------------------------------ |
| `AgentDefinitionError` | `agent_definition_error` | 配置、工具定义、运行选项非法               |
| `AgentMessageError`    | `agent_message_error`    | 消息结构/角色矩阵/JSON 输入非法（含 path） |
| `AgentModelError`      | `agent_model_error`      | 适配器失败、响应/流/usage 非法             |
| `AgentToolError`       | `agent_tool_error`       | 工具返回非法、审批回调失败                 |

## 与 config / log 配合

Agent 包不读取配置、不写日志。应用层可以：

```ts
import { createLogger } from '@kurumi/log';
import { loadConfig } from '@kurumi/config';

const logger = createLogger({ level: parseLogLevel(config.log_level) ?? 'info' });
const agent = createAgent({ provider, model: config.model });

for await (const event of agent.stream(messages)) {
  if (event.type === 'tool_end') {
    logger.info('tool finished', { tool: event.name, isError: event.result.isError });
  }
}
```

完整联动示例见 [`examples/deepseek-agent/src/verify.ts`](../../examples/deepseek-agent/src/verify.ts)。

## 脚本

```bash
pnpm --filter @kurumi/agent typecheck
pnpm --filter @kurumi/agent test
pnpm --filter @kurumi/agent test:coverage
pnpm --filter @kurumi/agent build
```

契约与追溯矩阵：[`docs/spec/agent.md`](../../docs/spec/agent.md)
