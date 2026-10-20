# @kurumi/frankfurter

Frankfurter（ECB 参考汇率）只读适配器：**即期汇率、全部汇率、ECB 货币列表与 FX 日线**。

## 数据源

- `https://api.frankfurter.app/latest`
- `https://api.frankfurter.app/currencies`
- `https://api.frankfurter.app/{start}..{end}`
- 无需 API Key；ECB 参考汇率通常每个工作日更新一次。

## 提供方

`createFrankfurterProvider(options)` 实现 `FxRateProvider`，并额外提供：

- `getRate(base, quote)`：单条汇率
- `listRates()`：基准货币的全部汇率（默认 EUR）
- `listCurrencies()`：ECB 支持的货币
- `getBars(fxInstrument, { interval: '1d', fromMs, toMs })`：FX 日线

> Frankfurter 只提供 ECB 参考汇率，不提供买卖盘/成交。因此 FX 日线的 `open/high/low/close` 均等于当日参考汇率，`volume = 0`；这是“参考汇率 K 线”，不是可交易 OHLC。

## 使用

```ts
import { createCurrency, createInstrument } from '@kurumi/finance';
import { Decimal } from '@kurumi/math';
import { createFrankfurterProvider } from '@kurumi/frankfurter';

const EUR = createCurrency('EUR', 2);
const USD = createCurrency('USD', 2);
const provider = createFrankfurterProvider();

const rate = await provider.getRate(EUR, USD);
const fx = createInstrument({
  assetClass: 'fx',
  venue: 'fx',
  symbol: 'EUR/USD',
  baseCurrency: EUR,
  quoteCurrency: USD,
  tickSize: Decimal.parse('0.0001'),
  lotSize: Decimal.parse('1000'),
});
const bars = await provider.getBars(fx, {
  interval: '1d',
  fromMs: Date.now() - 14 * 86_400_000,
  toMs: Date.now(),
});
```

## 限制

- 仅日线；不支持分钟/小时 K 线。
- 只有 ECB 参考汇率，不含点差、成交量与盘中高低点。
- 货币精度可通过 `currencyDecimals` 覆盖；未列出货币缺省 2 位。
- 可选 `fetchImpl`、`timeoutMs`、`headers` 注入。
