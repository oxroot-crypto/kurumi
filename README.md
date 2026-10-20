# kurumi

Kurumi monorepo。

## 包结构

| 包                  | 职责                                                                                 |
| ------------------- | ------------------------------------------------------------------------------------ |
| `@kurumi/core`      | 跨 runtime 基础设施：错误基类、JSON 值/错误序列化、`RuntimeHost` 抽象、Node/内存宿主 |
| `@kurumi/schema`    | 通用 schema：类型安全解析、结构化错误、敏感值遮蔽、JSON Schema 导出                  |
| `@kurumi/config`    | 配置系统：schema 校验、多层来源合并、环境变量映射、插值、可重载 store                |
| `@kurumi/log`       | 日志基础设施：级别过滤、结构化字段、可插拔 sink、Node 文件 sink                      |
| `@kurumi/agent`     | Agent SDK 基础设施：消息契约、模型/工具抽象、流式运行循环、事件、会话与重试          |
| `@kurumi/open-ai`   | OpenAI 协议兼容模型适配器：Chat Completions、工具调用、SSE 流式与 usage 映射         |
| `@kurumi/anthropic` | Anthropic Messages 协议适配器：工具调用、thinking、SSE 流式与 usage 映射             |
| `examples/*`        | 真实环境验证示例（DeepSeek + core/schema/config/log/agent/open-ai/anthropic 联动）   |
| `apps/*`            | 应用入口（待建设）                                                                   |

## 开发命令

```bash
pnpm install       # 安装工作区依赖
pnpm typecheck     # 所有包类型检查
pnpm test          # 所有包单元测试（含 fast-check 属性测试）
pnpm test:coverage # 单元测试 + 覆盖率阈值
pnpm lint          # oxlint
pnpm format        # prettier 写入
pnpm clean         # 清理 dist
pnpm build         # 按依赖顺序构建所有包

pnpm --filter @kurumi/core build
pnpm --filter @kurumi/schema build
pnpm --filter @kurumi/config build
pnpm --filter @kurumi/log build
pnpm --filter @kurumi/agent build
pnpm --filter @kurumi/open-ai build
pnpm --filter @kurumi/anthropic build
```

## 真实环境验证

`examples/deepseek-agent/` 通过 `.env` 中的 `KURUMI_API_KEY` 调用 DeepSeek，并串联 core/schema/config/log/agent/open-ai/anthropic：

```bash
cp examples/deepseek-agent/.env.example examples/deepseek-agent/.env   # 填入 KURUMI_API_KEY
pnpm --filter @kurumi/example-deepseek-agent verify                   # OpenAI 协议
pnpm --filter @kurumi/example-deepseek-agent verify:anthropic         # DeepSeek Anthropic 兼容 API
```

详见 [`examples/deepseek-agent/README.md`](examples/deepseek-agent/README.md)。

## 许可

[The Unlicense](LICENSE)（公共领域，无使用限制）。

## 文档

- core 契约：[`docs/spec/core.md`](docs/spec/core.md)
- schema 契约与追溯矩阵：[`docs/spec/schema.md`](docs/spec/schema.md)
- config 契约与追溯矩阵：[`docs/spec/config.md`](docs/spec/config.md)
- log 契约与追溯矩阵：[`docs/spec/log.md`](docs/spec/log.md)
- agent 契约与追溯矩阵：[`docs/spec/agent.md`](docs/spec/agent.md)
- open-ai 契约与追溯矩阵：[`docs/spec/open-ai.md`](docs/spec/open-ai.md)
- anthropic 契约与追溯矩阵：[`docs/spec/anthropic.md`](docs/spec/anthropic.md)
- schema 使用说明：[`packages/schema/README.md`](packages/schema/README.md)
- config 使用说明：[`packages/config/README.md`](packages/config/README.md)
- log 使用说明：[`packages/log/README.md`](packages/log/README.md)
- agent 使用说明：[`packages/agent/README.md`](packages/agent/README.md)
- open-ai 使用说明：[`providers/open-ai/README.md`](providers/open-ai/README.md)
- anthropic 使用说明：[`providers/anthropic/README.md`](providers/anthropic/README.md)
