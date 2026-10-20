# @kurumi/nasdaq 形式化契约

> 版本：0.1.0  
> 范围：`providers/nasdaq/src` 的 Nasdaq 公开行情、日线与股票期权链适配器。  
> 判定：本包是对外数据契约适配器，属于高可靠性边界模块；不涉及资金/交易执行，但必须保证只读、可取消、错误可定位。

## 1. 范围与职责

`@kurumi/nasdaq` 负责：

- 将 `api.nasdaq.com` 的公开 JSON 映射为 `@kurumi/finance` 的 `Ticker`、`Quote`、`Bar` 与期权链对象；
- 对价格按标 `tickSize` 对齐，对时间、价格、数量与盘口结构做入口校验；
- 将 HTTP/解析/字段错误统一包装为 `ProviderHttpError` / `ProviderDataError`。

不负责：账户、下单、撤单、交易执行、盘口增量、非日线 K 线或官方 OCC symbol 生成。

## 2. 形式化约束矩阵

| 编号                 | 约束                                                                                                                 | 失败语义                                                  |
| -------------------- | -------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------- |
| `FC-NASDAQ-PRE-001`  | `id` 非空；`baseUrl` 可解析；`timeoutMs` 为正安全整数；`maxRows` 为正安全整数；`assetClass ∈ {stocks, etf, index}`。 | `ProviderDataError`                                       |
| `FC-NASDAQ-PRE-002`  | `getBars` 只接受 `1d`；`fromMs`/`toMs` 为非负安全整数且 `toMs > fromMs`。                                            | `UnsupportedProviderOperationError` / `ProviderDataError` |
| `FC-NASDAQ-PRE-003`  | 所有 provider 方法接受可选 `AbortSignal` 并透传到底层 fetch。                                                        | —                                                         |
| `FC-NASDAQ-POST-001` | `getTicker`/`getQuote` 映射 `primaryData`；价格按 tick 对齐；bid/ask 必须满足 finance 的盘口约束。                   | `ProviderDataError`                                       |
| `FC-NASDAQ-POST-002` | `getBars` 返回按 `openTimeMs` 升序的日线，OHLC 按 tick 对齐，`closeTimeMs = openTimeMs + 1d`。                       | `ProviderDataError`                                       |
| `FC-NASDAQ-POST-003` | `getExpirations` 返回去重、升序的 UTC 到期日；`getChain` 按到期日筛选并生成稳定 `contractId`。                       | `ProviderDataError`                                       |
| `FC-NASDAQ-ERR-001`  | HTTP 非 2xx/网络失败为 `ProviderHttpError`；JSON/字段/日期解析失败为 `ProviderDataError`。                           | 对应 provider 错误                                        |
| `I-NASDAQ-001`       | 包内不存在 `submitOrder` / `cancelOrder` / `executeOrder` / `placeOrder` 等写操作。                                  | 架构测试失败                                              |

## 3. 边界与采样

- 日期：`MM/DD/YYYY` 与 `Month D, YYYY` 必须通过日历回环比对；拒绝 `02/31`、`13/01`。
- 价格：带 `$`、千分位与子最小变动价位；按 tick 使用 `half-even` 对齐。
- 期权：空到期日、缺失 bid/ask、重复行、未知 `expirygroup`、非正 strike。
- 网络：4xx/5xx、超时、取消、非 JSON、`data: null`。

## 4. 追溯矩阵

| 约束                                      | 测试                                                             |
| ----------------------------------------- | ---------------------------------------------------------------- |
| `FC-NASDAQ-PRE-*`                         | `src/support.test.ts`                                            |
| `FC-NASDAQ-POST-001` / `-002` / `ERR-001` | `src/market-data.test.ts`                                        |
| `FC-NASDAQ-POST-003`                      | `src/options-chain.test.ts`                                      |
| `I-NASDAQ-001`                            | `packages/finance/src/architecture.test.ts` 的 provider 只读扫描 |

## 5. 兼容性与演进

- 破坏性变更：`contractId` 格式、日期解析语义、tick 对齐规则、字段映射。
- 非破坏性变更：新增可选资产类别、可选字段、只读能力。
- 数据源没有正式 SLA；新增能力必须保持显式失败，不得静默返回空数据。
