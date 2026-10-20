---
'@kurumi/core': minor
'@kurumi/math': minor
'@kurumi/finance': minor
'@kurumi/finance-tools': minor
'@kurumi/nasdaq': minor
'@kurumi/binance': minor
'@kurumi/frankfurter': minor
'@kurumi/us-treasury': minor
'@kurumi/sec-edgar': minor
---

# 新增 `@kurumi/math` 与 `@kurumi/finance`

首个金融基础设施版本：固定精度小数与金额、跨资产标的、行情、订单状态机、成交、持仓、组合、FX、期权/衍生品、基本面、估值、固定收益、绩效风控、公司行为与只读数据提供方契约。

- 新增 `@kurumi/math`：定点小数 `Decimal`、统计与风险度量、CAPM/WACC/DCF/Gordon、Black-Scholes/Greeks/隐含波动率。
- finance 账务计算统一使用 `Decimal`（`bigint` + 十进制 scale），禁止浮点金额运算。
- 覆盖股票、加密货币、FX、期货、永续、期权与指数标的。
- 提供订单状态机、超量/限价/时序校验、持仓成本与已实现/未实现盈亏、组合现金流与多币种权益。
- 提供三大财务报表、财务比率、成长性、DuPont、Piotroski、相对估值、DCF/WACC/CAPM、债券 YTM/久期/凸性、绩效与 VaR/CVaR。
- provider 按行情、参考数据、基本面、FX、债券分领域，全部只读；不提供交易执行接口。
- 通用校验下沉到 `@kurumi/core`；finance 通过错误工厂绑定金融领域错误。
- 新增 `@kurumi/finance-tools`：provider 查询、复合分析与纯计算三类 Agent 工具，默认预设可通过分组开关裁剪。
- 新增只读金融数据 provider：
  - `@kurumi/nasdaq`：美股 Ticker、日线与股票期权链。
  - `@kurumi/binance`：现货、永续/期货行情与参考数据。
  - `@kurumi/frankfurter`：ECB 参考汇率与 FX 日线。
  - `@kurumi/us-treasury`：美国国债收益率曲线。
  - `@kurumi/sec-edgar`：美股参考数据、公司资料与财务报表。
- 新增 `examples/finance-agent`，真实调用上述公开数据源进行端到端验证。
