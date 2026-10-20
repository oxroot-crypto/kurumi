# @kurumi/us-treasury 形式化契约

> 版本：0.1.0  
> 范围：`providers/us-treasury/src` 的美国财政部每日国债收益率曲线适配器。  
> 判定：只读参考数据适配器；曲线是估值输入，不是交易报价。

## 1. 范围与职责

`@kurumi/us-treasury` 提供：

- `getLatest(currency)`：最近可用 USD 曲线；
- `getCurve(date, currency)`：指定 `YYYY-MM-DD` 的 USD 曲线。

不负责：个券价格、应计利息、债券交易、非 USD 曲线。

## 2. 形式化约束矩阵

| 编号                     | 约束                                                                                                                          | 失败语义            |
| ------------------------ | ----------------------------------------------------------------------------------------------------------------------------- | ------------------- |
| `FC-USTREASURY-PRE-001`  | `id` 非空；`baseUrl` 可解析；`timeoutMs` 为正安全整数；只接受 USD。                                                           | `ProviderDataError` |
| `FC-USTREASURY-PRE-002`  | `getCurve` 日期必须是合法日历 `YYYY-MM-DD`；`getLatest` 按当前年、前一年、前两年回退。                                        | `ProviderDataError` |
| `FC-USTREASURY-PRE-003`  | 所有异步方法接受可选 `AbortSignal` 并透传。                                                                                   | —                   |
| `FC-USTREASURY-POST-001` | CSV 列名映射为固定期限；收益率百分比转换为小数，scale 8、half-even；期限点按 `tenorYears` 升序。                              | `ProviderDataError` |
| `FC-USTREASURY-POST-002` | 返回冻结 `YieldCurve`；日期格式为 `YYYY-MM-DD`；空曲线显式失败。                                                              | `ProviderDataError` |
| `FC-USTREASURY-ERR-001`  | 非 2xx/网络失败为 `ProviderHttpError`；CSV 列数、日期、数值解析失败统一为 `ProviderDataError`，不得泄漏 `DecimalParseError`。 | 对应 provider 错误  |
| `I-USTREASURY-001`       | 包内不存在写操作或交易执行方法。                                                                                              | 架构测试失败        |

## 3. 边界与采样

- 空 CSV、只有表头、列数不一致、非法日期、`N/A`、非数值、负收益率。
- 跨年回退、指定日期无数据、节假日、取消信号。
- 期限列缺失或未知列应跳过；全部缺失时显式失败。

## 4. 追溯矩阵

| 约束                                                               | 测试                                        |
| ------------------------------------------------------------------ | ------------------------------------------- |
| `FC-USTREASURY-PRE-001` / `-002` / `POST-001` / `-002` / `ERR-001` | `src/provider.test.ts`                      |
| `FC-USTREASURY-PRE-003`                                            | `src/provider.test.ts` 的超时/取消覆盖      |
| `I-USTREASURY-001`                                                 | `packages/finance/src/architecture.test.ts` |

## 5. 兼容性与演进

- 破坏性变更：期限列映射、CSV 日期格式、收益率 scale、错误码。
- 非破坏性变更：新增期限点、可选注入。
- 仅 USD；其他币种必须新增独立 provider 与契约。
