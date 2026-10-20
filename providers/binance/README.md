# @kurumi/binance

Binance 公开数据只读适配器：**现货行情/参考数据 + 永续与期货行情/参考数据**。

## 数据源

- 现货：`https://api.binance.com/api/v3/*`
- 期货/永续：`https://fapi.binance.com/fapi/v1/*`
- 全部为公开市场数据接口，无需 API Key；不包含账户、下单、撤单或资金划转。
- Binance 有权重与频率限制；请遵守官方文档并自行做退避。

## 提供方

| 工厂                                        | 能力                                                          |
| ------------------------------------------- | ------------------------------------------------------------- |
| `createBinanceMarketDataProvider`           | 现货 quote/ticker/order book/bars/trades                      |
| `createBinanceReferenceDataProvider`        | 现货标的与货币，解析 `exchangeInfo` 的 tick/lot/minNotional   |
| `createBinanceFuturesMarketDataProvider`    | 永续/期货 quote/ticker/order book/bars/trades                 |
| `createBinanceFuturesReferenceDataProvider` | 永续/期货标的；`PERPETUAL` → `perpetual`，季度合约 → `future` |

## 使用

```ts
import { createCurrency, createInstrument } from '@kurumi/finance';
import { Decimal } from '@kurumi/math';
import { createBinanceMarketDataProvider } from '@kurumi/binance';

const USDT = createCurrency('USDT', 8);
const btc = createInstrument({
  assetClass: 'crypto',
  venue: 'binance',
  symbol: 'BTC/USDT',
  baseAsset: 'BTC',
  quoteCurrency: USDT,
  tickSize: Decimal.parse('0.01'),
  lotSize: Decimal.parse('0.001'),
});

const provider = createBinanceMarketDataProvider();
const quote = await provider.getQuote!(btc);
const bars = await provider.getBars!(btc, {
  interval: '1h',
  fromMs: Date.now() - 86_400_000,
  toMs: Date.now(),
});
```

## 限制

- 仅公开市场数据；不提供账户、订单、杠杆、保证金或资金费结算接口。
- `getBars` 单次最多 1000 根；需要更长区间时由调用方分段请求。
- 期货参考数据使用交易所 symbol（例如 `BTCUSDT`、`BTCUSDT_260116`）；现货使用 `BASE/QUOTE`。
- 货币精度可通过 `currencies` 覆盖；未知货币缺省 8 位。
- 可选 `fetchImpl`、`timeoutMs`、`headers` 注入。
