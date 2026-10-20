# @kurumi/finance-tools

Kurumi 的 Agent 金融工具集：**provider 查询工具 + 复合分析工具 + 纯计算工具**。

- **provider 无关**：工具包只依赖 `@kurumi/finance` 的 provider 接口，不导入任何具体数据源；Nasdaq/Binance/SEC 等由调用方注入。
- **分层工具**：
  - `finance_get_*`：低层只读查询工具（Ticker、Quote、盘口、K 线、成交、基本面、FX、期权链、收益率曲线）。
  - `finance_*_analysis`：一次完成“查询 + 计算 + 摘要”的复合工具。
  - `finance_*` 纯计算工具：金额、期权定价、债券、绩效、公司行为、持仓与风控。
- **默认预设可控**：`createFinanceTools` 默认包含三类工具；需要控制模型工具数量时可按分组关闭。
- **输入边界**：工具输入由 `@kurumi/schema` 校验；金额/价格使用十进制字符串，日期使用严格 `YYYY-MM-DD`。

## 安装与导入

```ts
import { createFinanceTools, createFinanceToolRegistry } from '@kurumi/finance-tools';
```

## 创建工具

```ts
const tools = createFinanceTools({
  instruments: [btc, aapl],
  marketData,
  fundamentalData,
  fxRates,
  fxBars,
  options,
  yieldCurve,
});

const registry = createFinanceToolRegistry({
  instruments: [btc, aapl],
  marketData,
  // 只保留复合分析与纯计算工具，减少模型选择成本
  shouldIncludeProviderTools: false,
});
```

### 分组开关

| 选项                          | 缺省   | 说明                                                          |
| ----------------------------- | ------ | ------------------------------------------------------------- |
| `shouldIncludeAnalysisTools`  | `true` | 纯计算工具（金额、期权、债券、绩效、持仓、风控等）            |
| `shouldIncludeCompositeTools` | `true` | `finance_market_analysis` 等复合工具                          |
| `shouldIncludeProviderTools`  | `true` | 低层 `finance_get_*` 查询工具；对应 provider 不存在时自动跳过 |

> 当 provider 能力完整时，默认预设约 26 个工具（8 个纯计算 + 5 个复合 + 13 个 provider 查询）。如果模型上下文紧张，建议关闭 `shouldIncludeProviderTools`，只保留复合工具。

## Provider 注入

工具包不绑定具体数据源。以 Nasdaq + SEC 为例：

```ts
import { createNasdaqMarketDataProvider } from '@kurumi/nasdaq';
import { createSecEdgarFundamentalProvider } from '@kurumi/sec-edgar';

const tools = createFinanceTools({
  instruments: [aapl],
  marketData: createNasdaqMarketDataProvider(),
  fundamentalData: createSecEdgarFundamentalProvider({
    userAgent: process.env.KURUMI_SEC_USER_AGENT!,
  }),
});
```

所有 provider 方法都是只读查询；不存在下单、撤单、资金划转或交易执行工具。

## 工具分组

### 纯计算

- `finance_money_calculate`
- `finance_option_price`
- `finance_option_implied_volatility`
- `finance_bond_analyze`
- `finance_performance_metrics`
- `finance_corporate_action_adjust`
- `finance_position_pnl`
- `finance_risk_check`

### 复合分析

- `finance_market_analysis`
- `finance_fundamental_analysis`
- `finance_fx_analysis`
- `finance_option_analysis`
- `finance_yield_curve`

### Provider 查询

按注入的 provider 能力动态生成：

- `finance_get_ticker` / `finance_get_quote` / `finance_get_order_book` / `finance_get_bars` / `finance_get_trades`
- `finance_list_instruments` / `finance_get_instrument`
- `finance_get_company_profile` / `finance_get_financial_statements`
- `finance_get_fx_rate` / `finance_list_fx_rates` / `finance_get_fx_bars`
- `finance_get_option_expirations` / `finance_get_options_chain`
- `finance_get_yield_curve`

## 设计约束

- 金额、价格、数量、费率与盈亏均通过 `Decimal` 十进制字符串传递，禁止用浮点数传金额。
- 日期输入使用严格 `YYYY-MM-DD`；非法日历日期（如 `2026-02-31`）会显式失败。
- 所有工具只读；工具结果会转换为 JSON 安全值，`Decimal`/`BigInt` 序列化为字符串。
- 工具失败通过 `FinanceToolError` 或 provider 结构化错误暴露，不静默返回空数据。
