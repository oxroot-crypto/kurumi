# kurumi

Kurumi monorepo。

## 包结构

| 包                      | 职责                                                                                      |
| ----------------------- | ----------------------------------------------------------------------------------------- |
| `@kurumi/core`          | 跨 runtime 基础设施：错误基类、通用校验、JSON 值/错误序列化、`RuntimeHost`、Node/内存宿主 |
| `@kurumi/schema`        | 通用 schema：类型安全解析、结构化错误、敏感值遮蔽、JSON Schema 导出                       |
| `@kurumi/config`        | 配置系统：schema 校验、多层来源合并、环境变量映射、插值、可重载 store                     |
| `@kurumi/log`           | 日志基础设施：级别过滤、结构化字段、可插拔 sink、Node 文件 sink                           |
| `@kurumi/agent`         | Agent SDK 基础设施：消息契约、模型/工具抽象、流式运行循环、事件、会话与重试               |
| `@kurumi/math`          | 数学基础设施：定点小数、统计与风险度量、估值模型、期权定价数学                            |
| `@kurumi/finance`       | 金融域基础设施：跨资产标的、行情/订单/持仓/组合、基本面/固收/公司行为/风控                |
| `@kurumi/finance-tools` | Agent 金融工具：行情/基本面/FX/期权/收益率复合分析与纯计算工具                            |
| `@kurumi/nasdaq`        | Nasdaq 美股行情与股票期权链只读适配器                                                     |
| `@kurumi/binance`       | Binance 现货/期货公开行情与参考数据只读适配器                                             |
| `@kurumi/frankfurter`   | Frankfurter/ECB FX 参考汇率与日线只读适配器                                               |
| `@kurumi/us-treasury`   | 美国财政部收益率曲线只读适配器                                                            |
| `@kurumi/sec-edgar`     | SEC EDGAR 公司资料、参考数据与财务报表只读适配器                                          |
| `@kurumi/open-ai`       | OpenAI 协议兼容模型适配器：Chat Completions、工具调用、SSE 流式与 usage 映射              |
| `@kurumi/anthropic`     | Anthropic Messages 协议适配器：工具调用、thinking、SSE 流式与 usage 映射                  |
| `examples/*`            | 真实环境验证示例（DeepSeek + Agent 工具循环 + 金融 provider + 公开数据源）                |
| `apps/*`                | 应用入口（待建设）                                                                        |

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
pnpm --filter @kurumi/math build
pnpm --filter @kurumi/finance build
pnpm --filter @kurumi/nasdaq build
pnpm --filter @kurumi/binance build
pnpm --filter @kurumi/frankfurter build
pnpm --filter @kurumi/us-treasury build
pnpm --filter @kurumi/sec-edgar build
pnpm --filter @kurumi/open-ai build
pnpm --filter @kurumi/anthropic build
```

## 真实环境验证

`examples/finance-agent/` 通过 `.env` 中的 `KURUMI_API_KEY` 调用 DeepSeek，并串联 Agent 工具循环、金融 provider 与公开数据源：

```bash
cp examples/finance-agent/.env.example examples/finance-agent/.env   # 填入 KURUMI_API_KEY 与 KURUMI_SEC_USER_AGENT
pnpm --filter @kurumi/example-finance-agent verify
```

验证会要求模型自主调用金融工具、所有工具调用成功且最终回答非空。详见 [`examples/finance-agent/README.md`](examples/finance-agent/README.md)。

## 许可

[The Unlicense](LICENSE)（公共领域，无使用限制）。

## 文档

- core 契约：[`docs/spec/core.md`](docs/spec/core.md)
- schema 契约与追溯矩阵：[`docs/spec/schema.md`](docs/spec/schema.md)
- config 契约与追溯矩阵：[`docs/spec/config.md`](docs/spec/config.md)
- log 契约与追溯矩阵：[`docs/spec/log.md`](docs/spec/log.md)
- agent 契约与追溯矩阵：[`docs/spec/agent.md`](docs/spec/agent.md)
- math 契约与追溯矩阵：[`docs/spec/math.md`](docs/spec/math.md)
- finance 契约与追溯矩阵：[`docs/spec/finance.md`](docs/spec/finance.md)
- open-ai 契约与追溯矩阵：[`docs/spec/open-ai.md`](docs/spec/open-ai.md)
- anthropic 契约与追溯矩阵：[`docs/spec/anthropic.md`](docs/spec/anthropic.md)
- schema 使用说明：[`packages/schema/README.md`](packages/schema/README.md)
- config 使用说明：[`packages/config/README.md`](packages/config/README.md)
- log 使用说明：[`packages/log/README.md`](packages/log/README.md)
- agent 使用说明：[`packages/agent/README.md`](packages/agent/README.md)
- math 使用说明：[`packages/math/README.md`](packages/math/README.md)
- finance 使用说明：[`packages/finance/README.md`](packages/finance/README.md)
- finance tools 使用说明：[`tools/finance/README.md`](tools/finance/README.md)
- 金融 provider / tools 契约：
  - [`docs/spec/nasdaq.md`](docs/spec/nasdaq.md)
  - [`docs/spec/binance.md`](docs/spec/binance.md)
  - [`docs/spec/frankfurter.md`](docs/spec/frankfurter.md)
  - [`docs/spec/us-treasury.md`](docs/spec/us-treasury.md)
  - [`docs/spec/sec-edgar.md`](docs/spec/sec-edgar.md)
  - [`docs/spec/finance-tools.md`](docs/spec/finance-tools.md)
- 金融 provider 使用说明：
  - [`providers/nasdaq/README.md`](providers/nasdaq/README.md)
  - [`providers/binance/README.md`](providers/binance/README.md)
  - [`providers/frankfurter/README.md`](providers/frankfurter/README.md)
  - [`providers/us-treasury/README.md`](providers/us-treasury/README.md)
  - [`providers/sec-edgar/README.md`](providers/sec-edgar/README.md)
- 金融 Agent 真实验证：[`examples/finance-agent/README.md`](examples/finance-agent/README.md)
- open-ai 使用说明：[`providers/open-ai/README.md`](providers/open-ai/README.md)
- anthropic 使用说明：[`providers/anthropic/README.md`](providers/anthropic/README.md)
