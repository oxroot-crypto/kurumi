# @kurumi/sec-edgar

SEC EDGAR 公开数据只读适配器：**美股参考数据、公司资料与财务报表**。

## 数据源

- 公司 ticker：`https://www.sec.gov/files/company_tickers.json`
- 公司 facts：`https://data.sec.gov/api/xbrl/companyfacts/CIK{10位}.json`
- 无需 API Key，但 SEC 要求 `User-Agent` 包含联系方式，并遵守 fair access（建议不超过 10 req/s）。

## 提供方

| 工厂                                | 能力                                                 |
| ----------------------------------- | ---------------------------------------------------- |
| `createSecEdgarReferenceProvider`   | `listInstruments`、`getInstrument`、`listCurrencies` |
| `createSecEdgarFundamentalProvider` | `getCompanyProfile`、`getFinancialStatements`        |

```ts
import {
  createSecEdgarFundamentalProvider,
  createSecEdgarReferenceProvider,
} from '@kurumi/sec-edgar';

const userAgent = process.env.KURUMI_SEC_USER_AGENT;
if (!userAgent) throw new Error('KURUMI_SEC_USER_AGENT is required');

const reference = createSecEdgarReferenceProvider({ userAgent });
const aapl = await reference.getInstrument('sec:AAPL');
if (!aapl) throw new Error('AAPL not found');

const fundamentals = createSecEdgarFundamentalProvider({ userAgent });
const profile = await fundamentals.getCompanyProfile?.(aapl);
const statements = await fundamentals.getFinancialStatements(aapl, { periods: 3 });
```

## 报表映射

- 从 `companyfacts` 的 `us-gaap` 标签提取收入、成本、营业利润、税费、净利润、资产负债与现金流。
- 资产/负债明细使用报告总额减已知明细得到 `otherCurrentAssets`、`otherNonCurrentAssets`、`otherCurrentLiabilities`、`otherNonCurrentLiabilities`；若为负则显式失败。
- 使用 `createFinancialStatements` 强制资产恒等式；不满足时抛 `ProviderDataError`。
- 支持 annual（10-K）与 quarterly（10-Q）；TTM 不直接支持。
- 缺失字段会显式失败，不会静默填 0；股本缺失时抛 `ProviderDataError`，避免生成失真的 EPS/市值。

## 限制

- 不提供价格行情；价格请使用 Nasdaq provider 或其它行情源。
- XBRL 标签因公司/行业而异；provider 覆盖常见 US-GAAP 标签，不能保证所有公司都能完整映射。
- 仅美股；CIK 可通过 `cikMap` 显式覆盖。
- 可选 `fetchImpl`、`timeoutMs`、`headers` 注入。
