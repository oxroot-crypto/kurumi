# finance-agent

完整的金融 Agent 真实环境验证：**DeepSeek + Agent 工具循环 + 金融 provider + 公开数据源**。

这个 example 取代了旧的 `deepseek-agent` 与 `finance-providers`，同时验证：

- `@kurumi/config` 的 schema 校验、JSONC 文件来源、环境变量覆盖与密钥脱敏；
- `@kurumi/core` 的 Node `RuntimeHost`；
- `@kurumi/log` 的结构化日志与级别过滤；
- `@kurumi/agent` 的模型调用、工具调用、事件流与最终回答；
- `@kurumi/finance-tools` 的复合工具与独立计算工具；
- `@kurumi/nasdaq`、`@kurumi/binance`、`@kurumi/frankfurter`、`@kurumi/us-treasury`、`@kurumi/sec-edgar` 的公开数据查询；
- `@kurumi/finance` / `@kurumi/math` 的行情、指标、基本面与收益率计算。

## 运行模式

真实 DeepSeek Agent：模型自主决定调用哪些金融工具，工具再访问真实公开数据源。

```bash
pnpm --filter @kurumi/example-finance-agent verify
```

验证通过后，Agent 的最终回答（`finalText`）会写入 `examples/finance-agent/report.md`；该文件是生成产物，已加入 `.gitignore`。

## 配置

配置由 `@kurumi/config` 统一加载，优先级从低到高：

1. `examples/finance-agent/agent.jsonc`（可选，提供默认值与 system prompt）；
2. `KURUMI_*` 环境变量（由 `node --env-file=.env` 注入）。

```bash
cp examples/finance-agent/.env.example examples/finance-agent/.env
```

```dotenv
# 必填：模型 API Key；也兼容 DEEPSEEK_API_KEY（agent.jsonc 插值）
KURUMI_API_KEY=
# 可选：覆盖 agent.jsonc 中的同名字段
KURUMI_BASE_URL=https://api.deepseek.com
KURUMI_MODEL=deepseek-chat
KURUMI_LOG_LEVEL=info
# SEC 要求 User-Agent 包含联系方式，必填
KURUMI_SEC_USER_AGENT=kurumi-finance-agent/0.1 your-email@example.com
```

可覆盖的完整字段：`api_key`、`base_url`、`model`、`sec_user_agent`、`temperature`、`max_steps`、`max_output_tokens`、`tool_concurrency`、`log_level`、`system`。环境变量使用 `KURUMI_` 前缀并转为大写，例如 `KURUMI_MAX_STEPS`、`KURUMI_TOOL_CONCURRENCY`。

验证过程使用 `@kurumi/log` 的结构化日志输出，不直接写 console；`api_key` 在启动日志中经 schema `redact()` 脱敏。

## 架构

```text
Agent (DeepSeek)
  └─ @kurumi/finance-tools
       ├─ finance_market_analysis      行情 + K 线 + 指标摘要
       ├─ finance_fundamental_analysis 报表 + 比率/成长/DuPont/Piotroski
       ├─ finance_fx_analysis          汇率 + FX 日线 + 指标
       ├─ finance_option_analysis      到期日 + ATM 附近期权摘要
       ├─ finance_yield_curve          收益率曲线 + 10Y
       ├─ finance_get_*                低层 provider 查询工具（行情/基本面/FX/期权/收益率）
       └─ 纯计算工具                   期权定价/债券/绩效/公司行为/持仓/风控
  └─ example 层 provider router
       ├─ nasdaq          → @kurumi/nasdaq
       ├─ binance         → @kurumi/binance (spot)
       ├─ binance-futures → @kurumi/binance (futures)
       ├─ sec             → @kurumi/sec-edgar
       ├─ fx              → @kurumi/frankfurter
       └─ yield curve     → @kurumi/us-treasury
```

工具包 `@kurumi/finance-tools` **不导入任何具体 provider**；provider 由本 example 按 `instrument.venue` 路由后注入。

## 工具设计原则

- 复合工具一次完成“查询 + 计算 + 摘要”，避免 Agent 在多轮 tool_use 中搬运大数组。
- 独立工具只做单一职责（例如 `finance_get_bars` 只返回原始 K 线，不计算指标）。
- 每个工具输入字段都有 `describe`，模型能理解参数含义。
- 所有工具只读，不包含下单/撤单/账户/资金划转。

## 数据源

| 领域       | Provider              | 公开源                      |
| ---------- | --------------------- | --------------------------- |
| 美股行情   | `@kurumi/nasdaq`      | `api.nasdaq.com`            |
| 股票期权   | `@kurumi/nasdaq`      | Nasdaq option-chain         |
| 加密现货   | `@kurumi/binance`     | `api.binance.com`           |
| 永续/期货  | `@kurumi/binance`     | `fapi.binance.com`          |
| FX         | `@kurumi/frankfurter` | `api.frankfurter.app` (ECB) |
| 美债       | `@kurumi/us-treasury` | `home.treasury.gov`         |
| 美股基本面 | `@kurumi/sec-edgar`   | `sec.gov` / `data.sec.gov`  |

公开接口没有交易级 SLA，可能限频或临时不可用；验证脚本会以非零状态退出并打印失败项。

## 预期输出（结构）

```text
INFO [finance-agent] 金融 Agent 启动 {"mode":"DeepSeek","toolCount":26,...}
INFO [finance-agent] 工具调用开始 {"tool":"finance_market_analysis",...}
INFO [finance-agent] 工具调用结束 {"tool":"finance_market_analysis","isError":false,...}
...
INFO [finance-agent] 金融 Agent 验证通过 {"toolCalls":N,"failedTools":0,...}
INFO [finance-agent] 最终回答 {"text":"..."}
```

数值、工具调用次数与最终回答会随模型/日期变化；验证只要求工具成功执行、失败数为 0、最终回答非空。

## 脚本

```bash
pnpm --filter @kurumi/example-finance-agent typecheck
pnpm --filter @kurumi/example-finance-agent verify
```
