# @kurumi/finance-tools 形式化契约

> 版本：0.1.0  
> 范围：`tools/finance/src` 的 Agent 金融工具定义、输入 schema 与 provider 注入。  
> 判定：对外 Agent 工具契约，属于高可靠性/对外契约类模块；输入边界、只读性与错误语义均为强制。

## 1. 范围与职责

`@kurumi/finance-tools` 提供三类工具：

1. `finance_*` 纯计算工具：金额、期权定价、债券、绩效、公司行为、持仓与风控；
2. `finance_*_analysis` 复合工具：一次性查询并计算行情/基本面/FX/期权/收益率摘要；
3. `finance_get_*` 低层 provider 查询工具：由调用方注入的只读 provider 动态生成。

工具包不导入任何具体 provider；不包含下单、撤单、账户或资金划转工具。

## 2. 形式化约束矩阵

| 编号                 | 约束                                                                                                                                                                         | 失败语义           |
| -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------ |
| `FC-FTOOLS-PRE-001`  | `createFinanceTools` 默认包含分析、复合与 provider 查询三类工具；可用 `shouldIncludeAnalysisTools` / `shouldIncludeCompositeTools` / `shouldIncludeProviderTools` 关闭分组。 | —                  |
| `FC-FTOOLS-PRE-002`  | provider 查询工具只在对应 provider 方法存在时生成；不得生成空壳工具。                                                                                                        | —                  |
| `FC-FTOOLS-PRE-003`  | 所有输入由 `@kurumi/schema` 校验；金额/价格为十进制字符串，日期为严格 `YYYY-MM-DD`，拒绝非法日历日期。                                                                       | 工具参数校验错误   |
| `FC-FTOOLS-PRE-004`  | 工具只依赖 `@kurumi/finance` provider 接口，不导入 `@kurumi/nasdaq` 等具体适配器。                                                                                           | 架构测试失败       |
| `FC-FTOOLS-POST-001` | 复合工具在无 K 线时返回 `indicators: null`/空数组，不得因合法空结果抛错。                                                                                                    | —                  |
| `FC-FTOOLS-POST-002` | 工具结果转换为 JSON 安全值；`Decimal`/`BigInt` 为字符串；危险键被丢弃。                                                                                                      | —                  |
| `FC-FTOOLS-POST-003` | 独立异步查询应并行执行；取消信号必须传递到 provider，包括 `options.signal` 形式的 FX K 线。                                                                                  | —                  |
| `FC-FTOOLS-ERR-001`  | 业务失败抛 `FinanceToolError` 或 provider 结构化错误；不得静默返回伪造数据。                                                                                                 | `FinanceToolError` |
| `I-FTOOLS-001`       | 包内不存在写操作或交易执行工具。                                                                                                                                             | 架构测试失败       |

## 3. 工具分组

| 分组          | 工具                                                                                                                                                                                                                           | 默认 |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---- |
| 分析          | `finance_money_calculate`、`finance_option_price`、`finance_option_implied_volatility`、`finance_bond_analyze`、`finance_performance_metrics`、`finance_corporate_action_adjust`、`finance_position_pnl`、`finance_risk_check` | 开启 |
| 复合          | `finance_market_analysis`、`finance_fundamental_analysis`、`finance_fx_analysis`、`finance_option_analysis`、`finance_yield_curve`                                                                                             | 开启 |
| Provider 查询 | `finance_get_*`、`finance_list_*`                                                                                                                                                                                              | 开启 |

## 4. 边界与采样

- 缺失标的/货币、未知 provider 能力、空 K 线、空期权链、无汇率。
- 非法十进制字符串（`1.`、`1e1001`）、非法日期（`2026-02-31`、`2026-13-01`）。
- provider 抛出 `ProviderHttpError`/`ProviderDataError`、取消信号、并发调用。
- 金额/价格不得用浮点数传递；工具结果不得包含循环引用。

## 5. 追溯矩阵

| 约束                                               | 测试                                       |
| -------------------------------------------------- | ------------------------------------------ |
| `FC-FTOOLS-PRE-001` / `-002` / `-003`              | `src/tools.test.ts`、`src/options.test.ts` |
| `FC-FTOOLS-PRE-004` / `I-FTOOLS-001`               | `src/architecture.test.ts`                 |
| `FC-FTOOLS-POST-001` / `-002` / `-003` / `ERR-001` | `src/tools.test.ts`                        |

## 6. 兼容性与演进

- 破坏性变更：工具名、输入 schema、默认工具分组、结果字段。
- 非破坏性变更：新增工具分组、可选 provider 能力、新增只读工具。
- 新增具体 provider 不得进入本包；由调用方注入。
