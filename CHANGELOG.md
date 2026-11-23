# Changelog

All notable changes to Kurumi are documented in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

No tagged releases exist yet. The current workspace is version `0.1.0`.

## [Unreleased]

### Added

- Ten-crate Cargo workspace with one-way dependencies and a feature-gated
  `kurumi` facade.
- `kurumi-core`: decimal money, validated scalar types, calendars, business-day
  and day-count conventions, ring buffers, and the shared `KurumiError`.
- `kurumi-math`: interpolation, root finding, random number generation,
  statistics, covariance and correlation, regression, and linear algebra.
- `kurumi-ta`: OHLCV bars plus O(1) streaming and batch technical indicators.
- `kurumi-pricing`: yield curves, cashflow discounting, fixed-rate bonds,
  Black-Scholes-Merton, CRR binomial pricing, and implied volatility.
- `kurumi-risk`: VaR, CVaR, performance and drawdown metrics, position sizing,
  and benchmark-relative metrics.
- `kurumi-portfolio`: `(T, N)` return panels, portfolio optimizers, risk
  decomposition, rebalancing, and rolling beta/correlation.
- `kurumi-backtest`: single- and multi-asset simulation, fee and slippage
  models, parameter grid search, and walk-forward analysis.
- `kurumi-exec`: Sans-I/O order state machine, reservation ledger, pre-trade
  risk gates, paper matching, audit journal, backtest runner, and an opt-in
  async live layer.
- `kurumi-feed`: asynchronous market-data runtime with Nasdaq ITCH, NCDS,
  Alpaca-compatible WebSocket connectors, reconnection, fan-out hub, and OHLCV
  bar aggregation.
- Runnable examples under `crates/kurumi/examples/`.
- English and Simplified Chinese README, contribution guide, code of conduct,
  security policy, and CI coverage for the MSRV and examples.

[Unreleased]: https://github.com/oxroot-crypto/kurumi/commits/main
