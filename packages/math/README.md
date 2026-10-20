# @kurumi/math

Kurumi 的跨 runtime 数学计算基础设施：**定点小数 + 统计与风险度量 + 估值模型 + 期权定价数学**。

- **定点小数**：`Decimal` 使用 `bigint` + 十进制 scale，适合价格、数量、金额与费率。
- **统计**：收益、年化、波动率、Sharpe/Sortino、回撤、Beta/Alpha、VaR/CVaR。
- **估值**：CAPM、WACC、DCF、Gordon、NPV/IRR、每股价值、安全边际。
- **期权数学**：内在价值、货币性、Black-Scholes、Greeks、隐含波动率。
- **无金融域依赖**：不导入 `@kurumi/finance`、不包含标的/订单/组合类型。
- **零第三方运行时依赖**：只依赖 `@kurumi/core`；主入口不导入 `node:`。

## 安装与导入

```ts
import {
  Decimal,
  DecimalRounding,
  simpleReturns,
  sharpeRatio,
  discountedCashFlow,
  OptionKind,
  blackScholes,
} from '@kurumi/math';
```

## Decimal

```ts
const price = Decimal.parse('100.25');
const quantity = Decimal.parse('2');
const notional = price.multiply(quantity); // 200.5

price.divide(Decimal.parse('3'), { scale: 18, rounding: 'half-even' });
price.toFixed(2, 'half-even');
price.rescale(2, 'exact'); // 无余数时成功，否则抛 DecimalRoundingError
```

- 零的 scale 为 0；非零值无尾随零；scale `0..30`。
- 乘法自然 scale 超过 30 时必须显式指定 `scale` 与 `rounding`。
- 除法默认 scale 18、`half-even`；`exact` 遇到非零余数直接失败。
- 支持 `down` / `up` / `ceil` / `floor` / `half-up` / `half-even` / `exact`。

## 统计与风险

```ts
const returns = simpleReturns([100, 110, 99, 120]);
annualizedReturn(returns, 12);
annualizedVolatility(returns, 12);
sharpeRatio(returns, 0.02, 12);
maxDrawdown([100, 110, 99, 120]); // -0.1
beta([0.1, 0.2, 0.3], [0.05, 0.1, 0.15]); // 2
historicalVaR(returns, 0.95);
expectedShortfall(returns, 0.95);
```

统计层使用 Float64；输出是模型/分析数值，不直接进入账务。

## 估值

```ts
const dcf = discountedCashFlow({
  cashFlows: [100, 110, 121],
  discountRate: 0.1,
  terminalGrowthRate: 0.02,
});

capitalAssetPricingModel({ riskFreeRate: 0.03, beta: 1.2, marketReturn: 0.08 });
weightedAverageCostOfCapital({
  equityValue: 600,
  debtValue: 400,
  costOfEquity: 0.1,
  costOfDebt: 0.05,
  taxRate: 0.25,
});
gordonGrowthModel({ dividendPerShare: 2, requiredReturn: 0.08, growthRate: 0.03 });
internalRateOfReturn([-100, 60, 60]);
```

## 期权数学

```ts
const result = blackScholes({
  spot: 100,
  strike: 100,
  timeToExpiryYears: 1,
  volatility: 0.2,
  riskFreeRate: 0.05,
  optionType: OptionKind.Call,
});
// result.price ≈ 10.4506

const iv = impliedVolatility(
  {
    spot: 100,
    strike: 100,
    timeToExpiryYears: 1,
    riskFreeRate: 0.05,
    optionType: OptionKind.Call,
  },
  result.price,
);
```

`Greeks` 返回 raw 单位：vega 对波动率 1.0、theta 对 1 年、rho 对利率 1.0。

## 与 finance 的关系

- `@kurumi/finance` 依赖本包，使用 `Decimal` 与期权/估值/统计数学。
- 金融域错误（例如 `FinanceDefinitionError`）仍属于 finance；本包使用 `MathDefinitionError` / `MathValueError`。
- finance 会重新导出常用数学能力以便使用，但实现与契约归属本包。

## 脚本

```bash
pnpm --filter @kurumi/math typecheck
pnpm --filter @kurumi/math test
pnpm --filter @kurumi/math test:coverage
pnpm --filter @kurumi/math build
```

契约：[`docs/spec/math.md`](../../docs/spec/math.md)
