# @kurumi/sec-edgar 形式化契约

> 版本：0.1.0  
> 范围：`providers/sec-edgar/src` 的 SEC EDGAR 参考数据、公司资料与 XBRL 财务报表适配器。  
> 判定：只读基本面适配器；报表映射属于高可靠性数据转换，缺失字段必须显式失败。

## 1. 范围与职责

`@kurumi/sec-edgar` 提供：

- 由 `company_tickers.json` 映射美股 `Instrument` 与 USD 货币；
- 公司资料（名称/国家/报表货币）；
- 由 `companyfacts` 构建 annual（10-K）与 quarterly（10-Q）`FinancialStatements`。

不负责：价格行情、TTM 合成、非 USD 报表、账户或交易执行。

## 2. 形式化约束矩阵

| 编号              | 约束                                                                                                 | 失败语义            |
| ----------------- | ---------------------------------------------------------------------------------------------------- | ------------------- |
| `FC-SEC-PRE-001`  | `userAgent` 必填且包含邮箱或 URL 联系方式；`baseUrl`/`tickersUrl` 可解析；`timeoutMs` 为正安全整数。 | `ProviderDataError` |
| `FC-SEC-PRE-002`  | 所有异步方法接受可选 `AbortSignal`；`periods` 为正安全整数；TTM 显式拒绝。                           | `ProviderDataError` |
| `FC-SEC-PRE-003`  | CIK 必须为 `0..10` 位数字并补齐 10 位；找不到 ticker/CIK 时显式失败。                                | `ProviderDataError` |
| `FC-SEC-POST-001` | 10-K/10-Q 期间按 `end` 去重，并选择 duration 最接近 365/90 天的 fact，避免把 YTD 当成单季度。        | `ProviderDataError` |
| `FC-SEC-POST-002` | 收入、净利润、资产、负债、权益与股本缺失时显式失败；不得回退为任意常量。                             | `ProviderDataError` |
| `FC-SEC-POST-003` | 资产负债表 residuals 不得为负；必须通过 finance 的资产恒等式校验。                                   | `ProviderDataError` |
| `FC-SEC-POST-004` | 返回按报告期从新到旧排列的冻结报表；日期必须是合法日历日期。                                         | `ProviderDataError` |
| `FC-SEC-ERR-001`  | 非 2xx/网络失败为 `ProviderHttpError`；JSON/字段/数值/日期解析失败为 `ProviderDataError`。           | 对应 provider 错误  |
| `I-SEC-001`       | 包内不存在写操作或交易执行方法。                                                                     | 架构测试失败        |

## 3. 边界与采样

- 缺失 start、缺失股本、非法数值、非法日期、10-Q 单季度与 YTD 同 end。
- 负税费、负权益、residuals 为负、缺少 Assets/Liabilities。
- 未知 ticker、非法 CIK、空 companyfacts、TTM 请求。
- 取消信号、超时、非 JSON、404。

## 4. 追溯矩阵

| 约束                                                        | 测试                                          |
| ----------------------------------------------------------- | --------------------------------------------- |
| `FC-SEC-PRE-001` / `-003` / `POST-004`                      | `src/support.test.ts`、`src/provider.test.ts` |
| `FC-SEC-PRE-002` / `POST-001` / `-002` / `-003` / `ERR-001` | `src/provider.test.ts`                        |
| `I-SEC-001`                                                 | `packages/finance/src/architecture.test.ts`   |

## 5. 兼容性与演进

- 破坏性变更：`periods` 语义、季度/年度 fact 选择、报表字段映射、CIK 解析。
- 非破坏性变更：新增 XBRL tag、可选公司资料字段。
- 缺失字段一律显式失败；不得通过默认值掩盖数据质量问题。
