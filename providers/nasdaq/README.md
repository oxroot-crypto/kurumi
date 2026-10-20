# @kurumi/nasdaq

Nasdaq 公开数据只读适配器：**美股 Ticker、日线历史与股票期权链**。

## 数据源

- 行情/历史：`https://api.nasdaq.com/api/quote/{symbol}/info`、`/historical`
- 期权链：`https://api.nasdaq.com/api/quote/{symbol}/option-chain`
- 无需 API Key；Nasdaq 公开接口没有正式 SLA，可能限频或调整字段。
- 数据可能延迟；仅用于研究与基础设施验证，不构成交易建议。

## 提供方

| 工厂                             | 能力                                                                    |
| -------------------------------- | ----------------------------------------------------------------------- |
| `createNasdaqMarketDataProvider` | `getTicker`、`getQuote`、`getBars`（仅 `1d`）；不提供 order book/trades |
| `createNasdaqOptionsProvider`    | `getExpirations`、`getChain`                                            |

## 使用

```ts
import { createCurrency, createInstrument } from '@kurumi/finance';
import { Decimal } from '@kurumi/math';
import { createNasdaqMarketDataProvider } from '@kurumi/nasdaq';

const USD = createCurrency('USD', 2);
const aapl = createInstrument({
  assetClass: 'equity',
  venue: 'nasdaq',
  symbol: 'AAPL',
  baseAsset: 'AAPL',
  quoteCurrency: USD,
  tickSize: Decimal.parse('0.01'),
  lotSize: Decimal.ONE,
});

const provider = createNasdaqMarketDataProvider();
const ticker = await provider.getTicker!(aapl);
const bars = await provider.getBars!(aapl, {
  interval: '1d',
  fromMs: Date.now() - 14 * 86_400_000,
  toMs: Date.now(),
});
```

期权链：

```ts
import { createNasdaqOptionsProvider } from '@kurumi/nasdaq';

const options = createNasdaqOptionsProvider({ assetClass: 'etf' });
const expirations = await options.getExpirations(spy);
const chain = await options.getChain(spy, expirations[0]!);
```

## 限制

- `getBars` 只支持日线；合并行情/盘前报价可能带子最小变动价位，provider 会对 Ticker、Quote 与 K 线按标的 `tickSize` 做 `half-even` 对齐。
- 期权链接口一次返回全部到期日，`getExpirations`/`getChain` 会拉取较大响应；可用 `maxRows` 控制。
- 期权 `expirationMs` 为 UTC 到期日零点；合约代码由 provider 生成，不是交易所官方 OCC symbol。
- 不支持盘口、逐笔成交、实时推送。
- 可选 `fetchImpl`、`timeoutMs`、`headers` 注入，便于测试与代理。

## 脚本

```bash
pnpm --filter @kurumi/nasdaq typecheck
pnpm --filter @kurumi/nasdaq test
pnpm --filter @kurumi/nasdaq build
```
