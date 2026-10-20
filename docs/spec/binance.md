# @kurumi/binance 形式化契约

> 版本：0.1.0  
> 范围：`providers/binance/src` 的 Binance 现货、永续/期货公开数据适配器。  
> 判定：本包是只读行情/参考数据适配器，属于高可靠性边界模块；禁止任何账户或交易执行能力。

## 1. 范围与职责

`@kurumi/binance` 提供：

- 现货与期货的 Quote、Ticker、OrderBook、Bar、AggTrade；
- 现货与期货 exchangeInfo 到 `Instrument` 的只读映射；
- 统一的 symbol 映射、货币精度、fetch/headers/timeout/signal 注入与错误包装。

不负责：账户、杠杆、保证金、资金费结算、下单/撤单、WebSocket 或历史分页聚合。

## 2. 形式化约束矩阵

| 编号                  | 约束                                                                                                                                    | 失败语义            |
| --------------------- | --------------------------------------------------------------------------------------------------------------------------------------- | ------------------- |
| `FC-BINANCE-PRE-001`  | `id` 非空；现货/期货 baseUrl 可解析；`timeoutMs` 为正安全整数；货币精度在 `0..18`。                                                     | `ProviderDataError` |
| `FC-BINANCE-PRE-002`  | 所有 provider 查询方法接受可选 `AbortSignal` 并透传到底层 fetch，包括参考数据的 `listInstruments`/`getInstrument`/`listCurrencies`。    | —                   |
| `FC-BINANCE-PRE-003`  | `getOrderBook.depth` 现货 `1..5000`、期货 `1..1000`；`getTrades.limit` 为 `1..1000` 的安全整数；`getBars` 的 `toMs` 必须大于 `fromMs`。 | `ProviderDataError` |
| `FC-BINANCE-POST-001` | symbol 映射移除 `/`、`-`，大写并去除 `PERP` 后缀；现货/期货使用各自 baseUrl。                                                           | —                   |
| `FC-BINANCE-POST-002` | K 线数组至少 9 项；`openTimeMs`、OHLCV、tradeCount 与 `closeTimeMs` 符合 finance `Bar` 约束。                                           | `ProviderDataError` |
| `FC-BINANCE-POST-003` | `aggTrades.m=true` 映射为卖方主动，否则为买方主动；tradeId 为字符串。                                                                   | —                   |
| `FC-BINANCE-POST-004` | exchangeInfo 映射为冻结 `Instrument`；参考数据方法返回按 `instrumentKey` 升序的数组。                                                   | `ProviderDataError` |
| `FC-BINANCE-ERR-001`  | 非 2xx/网络失败为 `ProviderHttpError`；JSON/字段/数值解析失败为 `ProviderDataError`。                                                   | 对应 provider 错误  |
| `I-BINANCE-001`       | 包内不存在任何写操作或交易执行方法。                                                                                                    | 架构测试失败        |

## 3. 边界与采样

- 空 symbols、非 TRADING、缺失 filters、`tickSize/stepSize` 为 0、`minQty > maxQty`。
- `lastUpdateId`、`closeTime`、`deliveryDate` 的非安全整数/负数。
- 空 bids/asks、交叉盘口、重复价格、0 数量。
- 取消信号在行情与参考数据两条路径都必须生效。

## 4. 追溯矩阵

| 约束                                                         | 测试                                                             |
| ------------------------------------------------------------ | ---------------------------------------------------------------- |
| `FC-BINANCE-PRE-001` / `POST-004` / `ERR-001`                | `src/provider.test.ts`                                           |
| `FC-BINANCE-PRE-002` / `-003` / `POST-001` / `-002` / `-003` | `src/provider.test.ts`、`src/support.test.ts`                    |
| `I-BINANCE-001`                                              | `packages/finance/src/architecture.test.ts` 的 provider 只读扫描 |

## 5. 兼容性与演进

- 破坏性变更：symbol 映射、`assetClass` 判定、合约类型、默认货币精度、错误码。
- 非破坏性变更：新增只读行情能力、可选配置字段。
- 不提供交易执行；未来新增写能力必须作为独立包与独立契约。
