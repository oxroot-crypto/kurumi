# @kurumi/us-treasury

美国财政部公开收益率曲线只读适配器：**每日国债收益率曲线**。

## 数据源

- `https://home.treasury.gov/resource-center/data-chart-center/interest-rates/daily-treasury-rates.csv/{year}/all?type=daily_treasury_yield_curve&field_tdr_date_value={year}&_format=csv`
- 无需 API Key；官方每日更新，节假日可能没有数据。

## 提供方

`createUsTreasuryProvider()` 实现 `YieldCurveProvider`：

- `getLatest(currency)`：最近可用曲线（当前年、前一年、前两年回退）
- `getCurve(date, currency)`：指定 `YYYY-MM-DD` 曲线
- 仅支持 USD

```ts
import { createCurrency } from '@kurumi/finance';
import { createUsTreasuryProvider } from '@kurumi/us-treasury';

const USD = createCurrency('USD', 2);
const provider = createUsTreasuryProvider();
const curve = await provider.getLatest(USD);
const tenYear = curve.points.find((point) => point.tenorYears === 10);
```

## 限制

- 仅 USD；其他货币请使用对应国家/地区的公开曲线。
- 仅日频；不提供债券个券、应计利息或交易价格。
- 期限点来自财政部固定列（1M/1.5M/2M/3M/4M/6M/1Y/2Y/3Y/5Y/7Y/10Y/20Y/30Y）。
- 可选 `fetchImpl`、`timeoutMs`、`headers` 注入。
