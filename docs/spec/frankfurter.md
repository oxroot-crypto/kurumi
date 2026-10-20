# @kurumi/frankfurter 形式化契约

> 版本：0.1.0  
> 范围：`providers/frankfurter/src` 的 ECB 参考汇率、货币列表与 FX 日线适配器。  
> 判定：只读参考数据适配器；参考汇率不是可交易报价，不构成交易建议。

## 1. 范围与职责

`@kurumi/frankfurter` 提供：

- `getRate(base, quote)` 与 `listRates()`；
- `listCurrencies()`；
- `getBars(fxInstrument, { interval: '1d', fromMs, toMs })`。

不负责：点差、成交量、盘中高低点、非日线周期、下单/兑换或资金划转。

## 2. 形式化约束矩阵

| 编号                      | 约束                                                                                                                | 失败语义                                                  |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------- |
| `FC-FRANKFURTER-PRE-001`  | `id` 非空；`baseUrl` 可解析；`baseCurrency` 合法；`currencyDecimals` 在 `0..18`。                                   | `ProviderDataError`                                       |
| `FC-FRANKFURTER-PRE-002`  | `getRate` 的 base/quote 不得相同；`getBars` 只接受 FX 标的与 `1d`，`fromMs/toMs` 为非负安全整数且 `toMs > fromMs`。 | `ProviderDataError` / `UnsupportedProviderOperationError` |
| `FC-FRANKFURTER-PRE-003`  | 所有异步方法接受可选 `AbortSignal`；`getBars` 同时兼容 `options.signal` 与第三个 `signal` 参数。                    | —                                                         |
| `FC-FRANKFURTER-POST-001` | 汇率解析为正 `Decimal`，日期解析为 UTC 毫秒，返回冻结 `FxRate`。                                                    | `ProviderDataError`                                       |
| `FC-FRANKFURTER-POST-002` | FX 日线 OHLC 均等于参考汇率、`volume = 0`；`openTimeMs` 必须位于 `[fromMs, toMs)`。                                 | `ProviderDataError`                                       |
| `FC-FRANKFURTER-POST-003` | API 的结束日期为包含语义，适配器必须回退 1ms 或过滤，保证 `toMs` 不含。                                             | —                                                         |
| `FC-FRANKFURTER-ERR-001`  | 非 2xx/网络失败为 `ProviderHttpError`；JSON/字段/汇率/日期解析失败为 `ProviderDataError`。                          | 对应 provider 错误                                        |
| `I-FRANKFURTER-001`       | 包内不存在写操作或交易执行方法。                                                                                    | 架构测试失败                                              |

## 3. 边界与采样

- `base=quote`、未知货币、非法日期、负汇率、空 rates。
- 周末/节假日无数据、起止同日、`toMs` 在盘中、取消信号。
- 返回顺序必须按 `openTimeMs` 升序。

## 4. 追溯矩阵

| 约束                                               | 测试                                        |
| -------------------------------------------------- | ------------------------------------------- |
| `FC-FRANKFURTER-PRE-001` / `POST-001` / `ERR-001`  | `src/provider.test.ts`                      |
| `FC-FRANKFURTER-PRE-002` / `POST-002` / `POST-003` | `src/provider.test.ts`                      |
| `FC-FRANKFURTER-PRE-003`                           | `src/provider.test.ts` 的取消/信号用例      |
| `I-FRANKFURTER-001`                                | `packages/finance/src/architecture.test.ts` |

## 5. 兼容性与演进

- 破坏性变更：参考汇率语义、`toMs` 边界、货币精度默认值。
- 非破坏性变更：新增只读查询、可选配置。
- 不引入点差/成交量等误导性字段；需要交易级 FX 时应使用独立可交易 provider。
