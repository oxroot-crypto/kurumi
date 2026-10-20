# @kurumi/finance

Kurumi 的跨 runtime 金融基础设施：**跨资产交易对象 + 基本面 + 固定收益 + 公司行为 + 风控 + 只读数据提供方契约**。定点数学、统计、估值与期权定价由 [`@kurumi/math`](../math/README.md) 提供。

- **账务不用浮点**：价格、数量、金额、费率、盈亏统一由 `Decimal`（`bigint` + 十进制 scale）表示。
- **跨资产**：股票、加密货币、FX、期货、永续、期权、指数。
- **完整交易链路**：标的规格、行情、订单状态机、成交、持仓、组合、FX、保证金、风控与撮合模拟。
- **完整分析链路**：三大财务报表、财务比率、成长性、DuPont、Piotroski、相对估值、DCF/WACC/CAPM、债券 YTM/久期/凸性、绩效与 VaR/CVaR。
- **不可变与显式失败**：工厂返回冻结对象；非法状态、超量成交、资产恒等式破坏、缺失汇率/mark price 均抛结构化错误。
- **provider 只读**：按行情、参考数据、基本面、FX、债券分领域；**不包含任何下单/撤单/执行接口**。
- **零第三方运行时依赖**：只依赖 `@kurumi/core` 与 `@kurumi/math`；主入口不导入 `node:`。

## 安装与导入

```ts
import {
  Decimal,
  createCurrency,
  createInstrument,
  createMoney,
  createQuote,
  createOrder,
  createFill,
  applyOrderEvent,
  applyFillToPortfolio,
  createPortfolio,
  computeFundamentalRatios,
  discountedCashFlow,
  createBondSpec,
  bondYield,
  sharpeRatio,
} from '@kurumi/finance';
```

## 分层与模块

| 层            | 模块                           | 职责                                                         |
| ------------- | ------------------------------ | ------------------------------------------------------------ |
| 数学          | `@kurumi/math`                 | 定点小数、统计、估值、期权定价数学                           |
| 金额          | `money.ts`                     | 货币、金额运算、格式化与 JSON DTO                            |
| 交易对象      | `instrument.ts`                | 标的判别联合、tick/lot、乘数、注册表、名义价值               |
| 行情          | `market-data.ts`               | Quote、Trade、Bar、Ticker、盘口快照与增量盘口                |
| 订单与成交    | `order.ts`                     | 订单构造、状态机、成交约束、平均成交价                       |
| 持仓与组合    | `position.ts` / `portfolio.ts` | 成本、已实现/未实现盈亏、现金、权益、敞口、多币种估值        |
| FX            | `fx.ts`                        | 直接/反向/多跳汇率、确定性 BFS、金额换算                     |
| 衍生品        | `option.ts` / `derivatives.ts` | 期权域封装、基差、资金费；定价数学由 `@kurumi/math` 提供     |
| 基本面        | `fundamentals.ts`              | 三大报表、财务比率、成长、DuPont、估值倍数、Piotroski        |
| 估值          | `@kurumi/math`                 | CAPM、WACC、DCF、Gordon、NPV/IRR、安全边际                   |
| 固定收益      | `fixed-income.ts`              | 付息日、应计利息、净价/全价、YTM、久期、凸性、DV01           |
| 绩效与风险    | `@kurumi/math`                 | 收益、年化、波动、Sharpe/Sortino、回撤、Beta/Alpha、VaR/CVaR |
| 公司行为      | `corporate-actions.ts`         | 拆股、股票股利、现金分红、历史价格/数量调整                  |
| 风控与撮合    | `risk.ts` / `simulation.ts`    | 下单限额、组合风险、保证金、强平、报价撮合                   |
| 只读 provider | `provider/`                    | 行情、参考数据、基本面、FX、债券查询契约                     |

## 快速开始：交易链路

```ts
import {
  Decimal,
  createCurrency,
  createInstrument,
  createMoney,
  createQuote,
  createOrder,
  createFill,
  acceptOrder,
  applyFill,
  createPosition,
  applyFillToPosition,
  createPortfolio,
  applyFillToPortfolio,
  portfolioEquity,
} from '@kurumi/finance';

const usdt = createCurrency('USDT', 8);
const btc = createInstrument({
  assetClass: 'crypto',
  venue: 'binance',
  symbol: 'BTC/USDT',
  baseAsset: 'BTC',
  quoteCurrency: usdt,
  tickSize: Decimal.parse('0.01'),
  lotSize: Decimal.parse('0.001'),
});

const quote = createQuote({
  instrument: btc,
  bid: Decimal.parse('60000'),
  ask: Decimal.parse('60001'),
  bidSize: Decimal.parse('1'),
  askSize: Decimal.parse('1'),
  timestampMs: 1_700_000_000_000,
});

const order = acceptOrder(
  createOrder({
    id: 'o-1',
    instrument: btc,
    side: 'buy',
    type: 'limit',
    quantity: Decimal.parse('0.01'),
    limitPrice: Decimal.parse('60001'),
    createdAtMs: 1_700_000_000_000,
  }),
  1_700_000_000_001,
);

const fill = createFill({
  id: 'f-1',
  orderId: 'o-1',
  instrument: btc,
  side: 'buy',
  price: quote.ask,
  quantity: Decimal.parse('0.01'),
  timestampMs: 1_700_000_000_002,
});

const nextOrder = applyFill(order, fill);
const position = applyFillToPosition(createPosition(btc), fill).position;
const portfolio = applyFillToPortfolio(
  createPortfolio(usdt, { cash: [createMoney(Decimal.parse('10000'), usdt)] }),
  fill,
).portfolio;

portfolioEquity(portfolio, {
  marks: { 'binance:BTC/USDT': Decimal.parse('61000') },
});
```

## 基本面

```ts
import {
  createFinancialStatements,
  computeFundamentalRatios,
  computeDuPont,
  computeValuationMultiples,
  computePiotroskiFScore,
} from '@kurumi/finance';

const statements = createFinancialStatements({
  period: {
    fiscalYear: 2025,
    periodType: 'annual',
    startMs: 1_735_689_600_000,
    endMs: 1_766_227_200_000,
    currency: usd,
  },
  income: {
    revenue: Decimal.parse('1000'),
    costOfRevenue: Decimal.parse('600'),
    operatingExpenses: Decimal.parse('200'),
    interestExpense: Decimal.parse('20'),
    taxExpense: Decimal.parse('36'),
    sharesOutstanding: Decimal.parse('100'),
  },
  balance: {
    cashAndEquivalents: Decimal.parse('100'),
    shortTermInvestments: Decimal.parse('50'),
    accountsReceivable: Decimal.parse('150'),
    inventory: Decimal.parse('100'),
    otherCurrentAssets: Decimal.parse('50'),
    propertyPlantEquipment: Decimal.parse('500'),
    goodwill: Decimal.parse('100'),
    intangibleAssets: Decimal.parse('50'),
    otherNonCurrentAssets: Decimal.parse('50'),
    accountsPayable: Decimal.parse('100'),
    shortTermDebt: Decimal.parse('50'),
    otherCurrentLiabilities: Decimal.parse('50'),
    longTermDebt: Decimal.parse('300'),
    otherNonCurrentLiabilities: Decimal.parse('50'),
    shareholdersEquity: Decimal.parse('600'),
  },
  cashFlow: {
    operatingCashFlow: Decimal.parse('180'),
    capitalExpenditure: Decimal.parse('50'),
    investingCashFlow: Decimal.parse('-100'),
    financingCashFlow: Decimal.parse('-20'),
    depreciationAmortization: Decimal.parse('40'),
    dividendsPaid: Decimal.parse('30'),
  },
});

computeFundamentalRatios(statements);
// currentRatio=2.25, netMargin=0.144, roe=0.24, ...

computeDuPont(statements);
computeValuationMultiples(statements, {
  price: Decimal.parse('20'),
  growthRate: Decimal.parse('0.1'),
});
computePiotroskiFScore(statements, priorStatements);
```

资产恒等式 `totalAssets == totalLiabilities + shareholdersEquity` 必须精确成立；利润表与现金流量表合计由原始行项目推导，避免调用方传入不一致的合计。

## 估值与固定收益

```ts
import {
  discountedCashFlow,
  weightedAverageCostOfCapital,
  createBondSpec,
  bondCleanPrice,
  bondYield,
  macaulayDuration,
  convexity,
  dv01,
} from '@kurumi/finance';

const dcf = discountedCashFlow({
  cashFlows: [100, 110, 121],
  discountRate: 0.1,
  terminalGrowthRate: 0.02,
});

const wacc = weightedAverageCostOfCapital({
  equityValue: 600,
  debtValue: 400,
  costOfEquity: 0.1,
  costOfDebt: 0.05,
  taxRate: 0.25,
});

const bond = createBondSpec({
  id: 'US-TEST-2030',
  faceValue: Decimal.parse('1000'),
  couponRate: Decimal.parse('0.05'),
  couponFrequency: 2,
  issueDateMs: Date.UTC(2020, 0, 1),
  maturityDateMs: Date.UTC(2030, 0, 1),
  dayCount: 'actual/actual',
  currency: usd,
});

const price = bondCleanPrice(bond, Date.UTC(2020, 0, 1), Decimal.parse('0.06'));
const ytm = bondYield(bond, Date.UTC(2020, 0, 1), price);
macaulayDuration(bond, Date.UTC(2020, 0, 1), ytm);
convexity(bond, Date.UTC(2020, 0, 1), ytm);
dv01(bond, Date.UTC(2020, 0, 1), ytm);
```

## 绩效与风险

```ts
import {
  simpleReturns,
  annualizedReturn,
  annualizedVolatility,
  sharpeRatio,
  maxDrawdown,
  beta,
  historicalVaR,
  expectedShortfall,
} from '@kurumi/finance';

const returns = simpleReturns([100, 110, 99, 120]);
annualizedReturn(returns, 12);
annualizedVolatility(returns, 12);
sharpeRatio(returns, 0.02, 12);
maxDrawdown([100, 110, 99, 120]); // -0.1
beta([0.1, 0.2, 0.3], [0.05, 0.1, 0.15]); // 2
historicalVaR(returns, 0.95);
expectedShortfall(returns, 0.95);
```

统计层使用 Float64；金额账务仍必须使用 `Decimal`。

## 只读数据提供方

`provider/` 按领域拆分，全部只读：

| 接口                      | 职责                                   |
| ------------------------- | -------------------------------------- |
| `MarketDataProvider`      | Quote、Ticker、OrderBook、Bars、Trades |
| `ReferenceDataProvider`   | 标的、货币等参考数据                   |
| `FundamentalDataProvider` | 公司资料、财务报表                     |
| `FxRateProvider`          | 单条/全部汇率                          |
| `BondDataProvider`        | 债券静态规格                           |

```ts
import type { MarketDataProvider } from '@kurumi/finance';

const provider: MarketDataProvider = {
  id: 'example',
  async getQuote(instrument) {
    /* 返回 createQuote(...) */
  },
  async getOrderBook(instrument) {
    /* 返回 createOrderBook(...) */
  },
  async getBars(instrument, options) {
    /* 返回 createBar(...)[] */
  },
};
```

**不提供** `submitOrder`、`cancelOrder`、`executeOrder`、账户划转或任何写操作。交易执行由上层在完成鉴权、幂等、风控与审计设计后自行实现。

## 设计约定

- `Decimal` 规范表示：零的 scale 为 0；无尾随零；scale `0..30`；`|units| < 10^1024`。
- 除法默认 scale 18、`half-even`；乘法在自然 scale 超限时必须显式指定 scale 与舍入模式；`exact` 模式遇到非零余数直接失败。
- 金额加减、比较必须同币种；跨币种必须显式经 `FxConverter`。
- 订单状态机终态不可再转移；成交不得超过剩余数量；限价单成交价不得劣于限价。
- 财务报表必须满足资产恒等式；比率分母不成立时字段为 `undefined`，不返回 `Infinity`/`NaN`。
- 债券 YTM 使用有界二分；未收敛或价格越界显式失败。
- 所有工厂返回递归冻结对象；不读取系统时间、不发网络请求、不写全局状态。

## 脚本

```bash
pnpm --filter @kurumi/finance typecheck
pnpm --filter @kurumi/finance test
pnpm --filter @kurumi/finance test:coverage
pnpm --filter @kurumi/finance build
```

契约与追溯矩阵：[`docs/spec/finance.md`](../../docs/spec/finance.md)  
数学契约：[`docs/spec/math.md`](../../docs/spec/math.md)
