# @kurumi/math 形式化契约

> 版本：0.1.0  
> 范围：`packages/math/src`（定点小数、统计与风险度量、估值模型、期权定价数学）。  
> 判定：本包承载资金/交易系统共用的数值与模型计算，属于高可靠性数值基础设施；Decimal 与统计/VaR 约束按强制处理，模型层浮点计算须明确标注边界。

## 1. 范围与职责边界

`@kurumi/math` 负责：

- 用十进制定点整数表示价格、数量、金额、费率与比率，禁止用浮点进行账务计算；
- 解析、规范化、四则运算、舍入、步长对齐与 Decimal 断言；
- 统计与风险度量：收益、年化、波动率、Sharpe/Sortino、回撤、Beta/Alpha、跟踪误差、VaR/CVaR；
- 估值模型：CAPM、WACC、DCF、Gordon、NPV/IRR、折现因子、每股价值与安全边际；
- 期权数学：内在价值、货币性、Black-Scholes、Greeks、隐含波动率；
- 技术指标：SMA/EMA/RSI/MACD/布林带/ATR/随机指标/VWAP。

`@kurumi/math` 不负责：

- 货币、金额、标的、订单、持仓、组合等金融域类型；
- 数据提供方、网络、存储、配置、日志；
- 结算/清算规则、税务、会计科目与交易执行；
- 依赖 `node:`、`console.*` 或第三方运行时库。

## 2. 类型与不变量

```text
Decimal            = { units: bigint, scale: int }       // 值 = units / 10^scale
DecimalRounding    = exact | down | up | ceil | floor | half-up | half-even
MaxDrawdownResult  = { maxDrawdown, peakIndex, troughIndex }
BlackScholesInput  = { spot, strike, timeToExpiryYears, volatility, riskFreeRate,
                       dividendYield?, optionType }
BlackScholesResult = { price, delta, gamma, vega, theta, rho }
OptionKind         = call | put
OhlcvPoint         = { timestampMs, open, high, low, close, volume }
TechnicalIndicatorSeries = { sma, ema, rsi, macd?, bollinger?, atr?, stochastic?, vwap? }
```

| 编号         | 不变量                                                                                                        |
| ------------ | ------------------------------------------------------------------------------------------------------------- |
| `I-MATH-001` | `Decimal` 始终规范化：零的 `scale = 0`；非零值无尾随零；`0 <= scale <= 30`；`abs(units) < 10^1024`。          |
| `I-MATH-002` | `Decimal` 值为不可变对象；所有运算返回新实例；不共享可变状态。                                                |
| `I-MATH-003` | 核心源码禁止用浮点四则运算计算 Decimal；浮点仅允许出现在统计、估值与期权定价模型。                            |
| `I-MATH-004` | 除法除数不得为零；`exact` 舍入遇到非零余数必须失败；不得静默丢失精度。                                        |
| `I-MATH-005` | 统计数据输入必须全部为有限数；价格必须 > 0；周期参数必须 > 0；置信度必须在 `(0,1)`。                          |
| `I-MATH-006` | 统计/估值/期权模型输出是模型数值，不得直接作为账务金额；金额账务必须转为 `Decimal` 并由上层显式承担舍入策略。 |
| `I-MATH-007` | 包主入口不得导入 `node:`、不得写 `console`；不依赖 `@kurumi/finance` 或任何 provider。                        |

## 3. 形式化约束矩阵

### 3.1 Decimal

| 编号                   | 约束                                                                                                                                                                                          | 失败语义                                              |
| ---------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------- |
| `FC-MATH-DEC-PRE-001`  | `parse` 只接受字符串；语法为可选符号 + 十进制数字 + 可选指数（`1.5`、`.5`、`-2e-3`）；拒绝空串、空白、`NaN`、`Infinity`、十六进制、下划线、`1.`、超长输入（> 2048 字符）与指数绝对值 > 1000。 | `DecimalParseError`                                   |
| `FC-MATH-DEC-PRE-002`  | 解析/构造结果必须满足 `0 <= scale <= 30`；指数展开后 scale 超限时拒绝；零值不受 scale 超限影响。                                                                                              | `DecimalOverflowError`                                |
| `FC-MATH-DEC-PRE-003`  | `fromUnits` 的 `scale` 必须是非负安全整数；`units` 必须是 `bigint`；规范化后绝对值必须 `< 10^1024`。                                                                                          | `DecimalOverflowError`                                |
| `FC-MATH-DEC-PRE-004`  | 除法除数不得为零；目标 scale 必须是 `0..30` 的安全整数。                                                                                                                                      | `DecimalDivisionByZeroError` / `DecimalOverflowError` |
| `FC-MATH-DEC-PRE-005`  | 舍入模式只能是 `exact` / `down` / `up` / `ceil` / `floor` / `half-up` / `half-even`；非法模式是定义错误；`exact` 遇到非零余数必须失败。                                                       | `MathDefinitionError` / `DecimalRoundingError`        |
| `FC-MATH-DEC-POST-001` | `parse(x).toString()` 幂等；输出为无指数、无尾随零的规范十进制文本。                                                                                                                          | —                                                     |
| `FC-MATH-DEC-POST-002` | 加法/减法对 scale 对齐后精确计算；乘法在自然 scale `<= 30` 时精确；除法默认 scale 18、`half-even`，显式 scale 优先。                                                                          | —                                                     |
| `FC-MATH-DEC-POST-003` | `compare` 只比较数值，不受 scale 表示影响；`equals` 等价于 `compare == 0`；`-0` 规范化为 `0`。                                                                                                | —                                                     |
| `FC-MATH-DEC-POST-004` | `toFixed(n)` 恰好输出 n 位小数；`toNumber` 明确为有损转换并拒绝溢出为 `±Infinity`。                                                                                                           | `DecimalOverflowError`                                |
| `FC-MATH-DEC-POST-005` | `roundDecimalToStep` 输出为 step 的整数倍；`isDecimalMultipleOf` 对任意符号正确判断整除。                                                                                                     | —                                                     |
| `FC-MATH-DEC-INV-001`  | 对任意 `a,b`：`a.add(b) == b.add(a)`；`a.subtract(b).add(b) == a`；`a.multiply(b) == b.multiply(a)`。                                                                                         | —                                                     |
| `FC-MATH-DEC-INV-002`  | 对任意可表示 `a,b != 0`：`a.divide(b,{scale:18,half-even}).multiply(b)` 与 `a` 的误差不超过 `abs(b) * 10^-18`。                                                                               | —                                                     |

### 3.2 统计与风险度量

| 编号                    | 约束                                                                                                                                      | 失败语义         |
| ----------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- | ---------------- |
| `FC-MATH-STAT-PRE-001`  | 价格序列长度 >= 2 且全部 > 0；收益率序列全部有限；周期参数为正有限数；置信度在 `(0,1)`；资产/基准序列长度相等且 >= 2。                    | `MathValueError` |
| `FC-MATH-STAT-POST-001` | 简单收益 `p_t/p_{t-1}-1`；对数收益 `ln(p_t/p_{t-1})`；累计收益 `Π(1+r)-1`；年化收益使用几何年化。                                         | —                |
| `FC-MATH-STAT-POST-002` | 年化波动率为样本标准差 * `sqrt(periodsPerYear)`；Sharpe 为年化超额收益 / 年化波动率；Sortino 使用下行偏差。                               | —                |
| `FC-MATH-STAT-POST-003` | 最大回撤返回负值（峰谷跌幅）及峰/谷索引；Calmar 为年化收益 / `abs(maxDrawdown)`。                                                         | —                |
| `FC-MATH-STAT-POST-004` | Beta = Cov(asset, benchmark)/Var(benchmark)；Alpha 为年化 Jensen Alpha；相关系数在 `[-1,1]`；跟踪误差为主动收益标准差 * `sqrt(periods)`。 | —                |
| `FC-MATH-STAT-POST-005` | 历史 VaR 为置信度分位损失的相反数（正数表示损失）；参数 VaR 使用 `1-confidence` 左尾正态分位；ES 为尾部平均损失；结果不修改输入。         | —                |
| `FC-MATH-STAT-INV-001`  | 对任意序列，`beta(series, series) == 1`（方差非零）；`correlation(series, series) == 1`；`trackingError(series, series) == 0`。           | —                |

### 3.3 估值模型

| 编号                   | 约束                                                                                                                                                 | 失败语义         |
| ---------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------- |
| `FC-MATH-VAL-PRE-001`  | CAPM：riskFreeRate/marketReturn 有限；beta 有限（可负）。                                                                                            | `MathValueError` |
| `FC-MATH-VAL-PRE-002`  | WACC：equityValue/debtValue >= 0 且不同时为 0；costOfEquity/costOfDebt/taxRate 有限，taxRate 在 `0..1`。                                             | `MathValueError` |
| `FC-MATH-VAL-PRE-003`  | DCF：cashFlows 非空且全部有限；discountRate 有限；terminalGrowthRate 与 terminalValue 不能同时提供；永续增长时 `discountRate > terminalGrowthRate`。 | `MathValueError` |
| `FC-MATH-VAL-PRE-004`  | Gordon：dividendPerShare >= 0、requiredReturn > growthRate、growthRate 有限。                                                                        | `MathValueError` |
| `FC-MATH-VAL-PRE-005`  | NPV/IRR：cashFlows 非空且有限；IRR 至少包含一正一负现金流，并在 `[-0.99, 10]` 内有界二分收敛。                                                       | `MathValueError` |
| `FC-MATH-VAL-POST-001` | CAPM 返回 `rf + beta * (market - rf)`；WACC 返回 `E/V*Ke + D/V*Kd*(1-tax)`；结果有限。                                                               | —                |
| `FC-MATH-VAL-POST-002` | DCF 返回各期现金流现值、终值现值与企业价值；终值使用 `lastCF*(1+g)/(r-g)` 或显式 terminalValue。                                                     | —                |
| `FC-MATH-VAL-POST-003` | Gordon 返回 `D1/(r-g)`；`perShareValue` 返回 `equityValue/shares`；`marginOfSafety = (intrinsic - price)/intrinsic`。                                | —                |
| `FC-MATH-VAL-POST-004` | NPV 为 `Σ CF_t/(1+r)^t`；IRR 使 NPV 在容差内为 0；结果不修改输入。                                                                                   | —                |

### 3.4 期权定价数学

| 编号                   | 约束                                                                                                                                       | 失败语义                              |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------- |
| `FC-MATH-OPT-PRE-001`  | 期权标的价格 > 0；strike > 0；Black-Scholes 的 spot/strike > 0，T >= 0，volatility >= 0，利率有限；optionType 只能是 call/put。            | `MathValueError` / `OptionModelError` |
| `FC-MATH-OPT-POST-001` | `optionIntrinsicValue`：call = max(S-K,0)，put = max(K-S,0)；`optionMoneyness = S/K`（scale 8、half-even）；实值判断严格大于/小于。        | —                                     |
| `FC-MATH-OPT-POST-002` | `optionBreakeven`：call = strike + premium，put = strike - premium；premium >= 0。                                                         | —                                     |
| `FC-MATH-OPT-POST-003` | Black-Scholes 在 T=0 或 sigma=0 时退化为无套利内在价值；正常参数下满足 put-call parity `C-P = S*e^{-qT} - K*e^{-rT}`（数值误差 <= 1e-4）。 | —                                     |
| `FC-MATH-OPT-POST-004` | Greeks 返回 raw 单位：vega 对波动率 1.0、theta 对 1 年、rho 对利率 1.0。                                                                   | —                                     |
| `FC-MATH-OPT-POST-005` | `impliedVolatility` 使用有界二分；市场价必须位于无套利上下界内；未收敛或越界抛错。                                                         | `OptionModelError`                    |
| `FC-MATH-OPT-INV-001`  | 对任意有效参数，`impliedVolatility(input, blackScholesPrice(input))` 在容差内还原输入波动率。                                              | —                                     |

### 3.5 技术指标

| 编号                   | 约束                                                                                                                                                                                                   | 失败语义         |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------- |
| `FC-MATH-IND-PRE-001`  | OHLCV 必须非空，open/high/low/close/volume 有限，volume >= 0，high >= max(open, close, low)，low <= min(open, close)。                                                                                 | `MathValueError` |
| `FC-MATH-IND-PRE-002`  | 所有周期为正安全整数；MACD 的 fast < slow；布林带 multiplier > 0；高/低/收盘序列长度一致。                                                                                                             | `MathValueError` |
| `FC-MATH-IND-POST-001` | SMA/EMA/RSI/MACD/布林带/ATR/随机指标/VWAP 输出与输入等长；warm-up 期为 null。                                                                                                                          | —                |
| `FC-MATH-IND-POST-002` | SMA 为窗口算术平均；EMA 以 SMA 为种子、`k=2/(n+1)`；RSI 使用 Wilder 平滑；MACD 为 fast EMA - slow EMA，信号线为 MACD 的 EMA，柱状图为差值。                                                            | —                |
| `FC-MATH-IND-POST-003` | 布林带中轨为 SMA，上下轨为 `middle ± multiplier * 总体标准差`；带宽 `(upper-lower)/middle`，`%B=(close-lower)/(upper-lower)`。                                                                         | —                |
| `FC-MATH-IND-POST-004` | ATR 使用真实波幅 `max(high-low,                                                                                                                                                                        | high-prevClose   | ,   | low-prevClose | )`与 Wilder 平滑；随机指标`%K=(close-lowest)/(highest-lowest)*100`，`%D`为`%K` 的 SMA。 | —   |
| `FC-MATH-IND-POST-005` | VWAP 为累计 `典型价*volume / 累计 volume`；累计 volume 为 0 时输出 null。`computeTechnicalIndicators` 默认包含 SMA[5,10,20]、EMA[12,26]、RSI14、MACD(12,26,9)、布林(20,2)、ATR14、随机(14,3) 与 VWAP。 | —                |

## 4. 错误分类

| 类型 / code                                               | 触发条件                             | 关键上下文           |
| --------------------------------------------------------- | ------------------------------------ | -------------------- |
| `MathError` / `math_error`                                | 数学包错误基类                       | `cause`              |
| `MathDefinitionError` / `math_definition_error`           | 工厂/选项/Decimal 操作数非法         | 直接消息             |
| `MathValueError` / `math_value_error`                     | 统计/估值/期权参数、序列、置信度非法 | 直接消息             |
| `DecimalParseError` / `decimal_parse_error`               | 十进制字符串语法非法                 | `input`              |
| `DecimalRoundingError` / `decimal_rounding_error`         | 精确舍入遇到非零余数                 | `operation`、`scale` |
| `DecimalDivisionByZeroError` / `decimal_division_by_zero` | 除数为零                             | `operation`          |
| `DecimalOverflowError` / `decimal_overflow`               | scale/位数超出上限                   | 直接消息             |
| `OptionModelError` / `option_model_error`                 | 定价参数/隐含波动率越界或未收敛      | 直接消息             |

## 5. 边界与采样

- Decimal：空串、空白、`+`/`-`、`.5`、`1.`、`1e-31`、`1e1000`、`NaN`、`Infinity`、`0x10`、`1_000`、超长输入、scale 0/30/31、负数、零、半偶舍入平局、非终止除法。
- 统计：空序列、单元素、全零收益、全负收益、零波动、置信度 0/1、基准方差 0、价格 0/负数、长度不一致。
- 估值：零/负折现率、终值增长率 >= 折现率、IRR 无解/多解边界、WACC 零权重、Gordon `r <= g`。
- 期权：T=0、sigma=0、深度实值/虚值、平价期权、隐含波动率上下界、未收敛、负波动率。
- 技术指标：短序列、全平价格、零成交量、周期大于序列、fast >= slow、OHLC 关系非法、warm-up 边界。

## 6. 追溯矩阵

| 约束                                                         | 测试文件 / 用例                                                             |
| ------------------------------------------------------------ | --------------------------------------------------------------------------- |
| `FC-MATH-DEC-*` / `I-MATH-001` / `I-MATH-002` / `I-MATH-004` | `src/decimal.test.ts`、`src/decimal.properties.test.ts`                     |
| `FC-MATH-STAT-*` / `I-MATH-005`                              | `src/statistics.test.ts`                                                    |
| `FC-MATH-VAL-*`                                              | `src/valuation.test.ts`                                                     |
| `FC-MATH-OPT-*`                                              | `src/option-math.test.ts`                                                   |
| `FC-MATH-IND-*`                                              | `src/indicators.test.ts`                                                    |
| `I-MATH-003`                                                 | `src/decimal.test.ts`（无浮点账务）、`src/architecture.test.ts`（模块隔离） |
| `I-MATH-006`                                                 | `src/option-math.test.ts`、`src/statistics.test.ts`（模型输出边界）         |
| `I-MATH-007`                                                 | `src/architecture.test.ts` · 无 `node:` / `console` / finance 依赖          |

## 7. 兼容性与演进

- 破坏性变更：修改 `Decimal` 规范表示、默认除法 scale/舍入、`DecimalRounding` 枚举值、统计函数定义（样本/总体、年化口径）、`OptionKind`、Black-Scholes 单位约定、错误码。
- 非破坏性变更：新增可选函数/字段、新增统计指标、新增期权模型（必须提供独立函数与契约）。
- `Decimal` 的 `units/scale` 是公开表示；调整规范化规则必须升 major 并迁移持久化数据。
- 统计/估值/期权模型使用 Float64；若未来改为 Decimal 精确模型，必须新增函数而不是静默替换现有语义。
- 契约与代码不一致时以本文件为唯一真实源，先更新契约再改实现。
