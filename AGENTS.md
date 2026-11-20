# Project: Kurumi (Rust All-in-One Financial Library)

## 1. Role & Objective

You are a Principal Quantitative Developer and Rust Systems Architect building
`kurumi`, a high-performance, institutional-grade financial library in Rust
(Edition 2024). Correctness, numerical stability, and explicit error handling
take priority over cleverness or premature micro-optimization.

## 2. Repository Layout

Kurumi is a single Cargo workspace. Every library lives under `crates/`:

| Crate | Responsibility |
| --- | --- |
| `kurumi-core` | Core primitives: money, high-precision numerics, calendars and day-count conventions, fixed-capacity ring buffers, the shared error type. |
| `kurumi-math` | Numerical methods: interpolation, root finding, random number generation, statistics, correlation and covariance estimation, regression, linear algebra. |
| `kurumi-ta` | Time series and technical indicators: candle structures, O(1) streaming incremental indicators, vectorized batch indicators. |
| `kurumi-pricing` | Asset and derivative pricing: cashflows, yield curves, bonds, option pricing engines. |
| `kurumi-risk` | Risk and performance analytics: VaR, CVaR, Greeks aggregation, Sharpe ratio, drawdowns, and benchmark-relative metrics. |
| `kurumi-portfolio` | Multi-asset panel alignment, return matrices, portfolio performance and risk decomposition, rolling beta/correlation panels. |
| `kurumi-backtest` | Strategy backtesting: order-book matching simulation, slippage and fee models, account/position tracking, and multi-asset portfolio backtesting under `portfolio`. |
| `kurumi-feed` | Asynchronous real-time market data: provider connectors, a resilience runtime, a multi-provider fan-out hub, and streaming OHLCV bar aggregation. Depends only on `kurumi-core` and `kurumi-ta`. |
| `kurumi` | Top-level facade: re-exports the synchronous member crates behind Cargo features and hosts the Python/WASM bindings. |

### Crate layering

Dependencies flow in one direction only:

```text
kurumi-core  <  kurumi-math  <  kurumi-ta / kurumi-pricing  <  kurumi-risk  <  kurumi-portfolio  <  kurumi-backtest  <  kurumi

kurumi-core  <  kurumi-ta  <  kurumi-feed          (isolated asynchronous branch)
```

A synchronous crate may depend only on crates that appear earlier in the main
chain. `kurumi-ta` and `kurumi-pricing` are peers: neither may depend on the
other. `kurumi-backtest` sits directly above `kurumi-portfolio` so multi-asset
backtesting can consume the return panels and optimizers it defines; the
dependency never points the other way. `kurumi-core` has no workspace
dependencies. `kurumi-feed` is an isolated asynchronous branch: it may depend on
`kurumi-core` and `kurumi-ta`, but no synchronous crate may depend on it. The
`kurumi` facade depends on every synchronous member crate, and no member may
depend on the facade. `kurumi-feed` is not part of the default facade feature
set; expose it, if ever, only behind an additive opt-in feature so a default
`kurumi` build never pulls an async runtime.

Crate boundaries are part of the public architecture: do not merge crates,
introduce a new crate, or add an edge that reverses the layering without an
explicit architectural decision recorded in the PR.

## 3. Architectural Invariants

- **Workspace structure**: maintain the nine crates and the one-way layering
  above, including the isolated `kurumi-feed` branch. No dependency cycles.
- **Dual numerical precision**:
  - Use `rust_decimal::Decimal` for currency amounts, ledger accounting, order
    prices, and discrete cashflows.
  - Use `f64` for continuous pricing models (Black-Scholes, Monte Carlo),
    root-finding, and statistical indicators.
  - Reject `NaN` and infinite values at every public API boundary using typed
    errors; never let them silently propagate.
- **Zero-panic policy**: never use `.unwrap()`, `.expect()`, or array/slice
  indexing `[]` in library code. Every fallible operation must return
  `Result<T, KurumiError>` (or `Result<T, FeedError>` in `kurumi-feed`). Use
  `get`, `checked_*`, and `try_*` APIs instead. Assertions are appropriate in
  tests, not on library paths. Member crates deny `clippy::unwrap_used`,
  `clippy::expect_used`, and `clippy::indexing_slicing` outside `cfg(test)`;
  any scoped `#[allow(...)]` needs a written justification.
- **Borrow checker simplicity**: prefer `Copy` value types (`Price`, `Rate`,
  `Date`, `Currency`) over complex reference lifetimes, and avoid
  self-referential structs.
- **Error discipline**: `KurumiError` (in `kurumi-core`) is the single public
  error type for the synchronous crates, and `FeedError` (in `kurumi-feed`) is
  the single public error type for the asynchronous market-data layer. Add
  variants with a descriptive `#[error("...")]` message, keep the originating
  context, and do not leak dependency error types through public APIs.
- **Safety**: library code is safe Rust. `unsafe_code` is denied workspace-wide;
  any unavoidable exception needs a scoped `#[allow(unsafe_code)]`, a `// SAFETY:`
  comment covering every invariant, and review justification.
- **Facade features**: `kurumi` exposes each synchronous member behind an
  additive feature (`core`, `math`, `ta`, `pricing`, `risk`, `portfolio`,
  `backtest`). Features are additive: enabling one must not break another, and
  lower-layer features are implied by higher-layer ones. Default features enable
  the whole synchronous library. `kurumi-feed` stays outside the default feature
  set so a default build never pulls an async runtime. Keep the feature graph in
  sync with the crate layering.
- **Bindings**: Python/WASM bindings live in `kurumi` behind dedicated opt-in
  features and must be thin adapters over the Rust API. Business logic never
  lives in binding code.

## 4. Multi-Asset & Matrix Conventions

These conventions are part of the public contract for every API that consumes
multi-asset return data or returns matrices, including the multivariate
statistics in `kurumi-math` and the benchmark-relative metrics in `kurumi-risk`.

- **Matrix orientation**: all multi-asset return matrices
  (`nalgebra::DMatrix<f64>`) MUST use shape `(T, N)`, where rows `T` are
  chronological time observations and columns `N` are assets. Never guess or
  silently transpose an input; a function that accepts a matrix MUST document
  the orientation it expects. Covariance and correlation matrices MUST be
  `(N, N)` symmetric matrices.
- **Degrees of freedom**: sample variance, covariance, and Pearson correlation
  MUST use Bessel's correction (`ddof = 1`, dividing by `T - 1`) by default,
  and MUST return `KurumiError::InsufficientData` when `T` is less than 2.
  Regression residual standard errors use the residual degrees of freedom.
  Model-based estimators such as EWMA and shrinkage covariance MUST document
  their weighting and normalization convention instead of inheriting the sample
  rule implicitly.
- **Numerical stability**: validate covariance matrices for symmetry and
  non-negative diagonals before consuming them. Clamp correlations to
  `[-1.0, 1.0]` and pin correlation diagonals to exactly `1.0`. A zero-variance
  series makes correlation, Beta, and regression slopes undefined: return a
  typed `KurumiError` rather than letting `NaN` or infinity reach
  correlation/Beta results.
- **Input alignment**: multi-series APIs MUST reject inputs of unequal length
  and non-finite values before computing. Callers align series (and perform any
  forward-fill or resampling) explicitly; library code never hides it.
- **Pure Rust solvers**: implement portfolio optimizers and multivariate
  regressions in pure Rust using `nalgebra`, with no C/C++/Fortran FFI
  dependency. Multivariate OLS MUST solve via SVD rather than the normal
  equations so that collinear regressors stay numerically stable.
- **Annualization**: benchmark-relative ratios conventionally quoted on an
  annual basis (tracking error, information ratio, Treynor ratio) MUST take an
  explicit `periods_per_year` and document the convention; per-period measures
  such as Jensen's alpha MUST state that they are per-period.

## 5. Multi-Asset Backtesting Invariants

- **Backward Compatibility**: Do NOT break existing single-asset
  `kurumi-backtest` public APIs or unit tests. Place multi-asset backtesting
  under `kurumi-backtest::portfolio`.
- **Deterministic Iteration**: ALWAYS use `std::collections::BTreeMap` (never
  `HashMap`) for positions, market snapshots, and order batches so backtest
  results are 100% deterministic.
- **Sell-Before-Buy Execution**: Rebalance order execution MUST settle all SELL
  orders first to replenish cash before executing BUY orders. Account for fees
  and slippage via a configurable cash reserve buffer (e.g.,
  `cash_buffer_ratio`) or pro-rata buy scaling so valid full-investment
  rebalances never fail due to rounding/fee shortfalls.
- **Zero Look-Ahead Bias**: Rolling portfolio optimizers MUST only receive
  historical prices/returns up to and including timestamp `t` (for
  `SignalBarClose`) or strictly before `t` when filling at open.
- **Decimal Accounting**: Cash, share quantities, fill prices, fees, slippage,
  and portfolio equity MUST remain `rust_decimal::Decimal`. Convert to `f64`
  only when constructing `ReturnMatrix` for `kurumi-portfolio` optimizers.

## 6. Parallel Validation & Walk-Forward Invariants

- **Feature-Gated Parallelism**: Gate all `rayon` usage behind
  `#[cfg(feature = "parallel")]` in `kurumi-backtest`, with sequential iterator
  fallbacks when the feature is disabled. Verify both `--all-features` and
  `--no-default-features` compile cleanly.
- **Zero-Copy Shared Market Data**: Parallel grid search and Walk-Forward folds
  MUST borrow immutable slices (`&[MarketSnapshot]`) backed by
  `AlignedMarketData` rather than cloning historical bar vectors per parameter
  combination.
- **Strict Determinism**: Parameter ranking and best-parameter selection MUST
  filter out non-finite (`NaN`/`Infinite`) scores and break ties deterministically
  by original grid index so parallel and sequential runs produce bit-for-bit
  identical winners.
- **No OOS Cold-Start & No Look-Ahead**: Walk-Forward OOS evaluation MUST support
  warmup prefix bars that populate indicator/lookback buffers without placing
  trades or recording equity before `eval_start_idx`, and IS parameter selection
  MUST NEVER observe any bar at or after `oos_start_idx`.

## 7. Real-Time Market Data (`kurumi-feed`) Invariants

`kurumi-feed` is the only asynchronous crate and the only place where a network
runtime is allowed. It is an isolated branch of the dependency graph: it may
depend on `kurumi-core` and `kurumi-ta`, and no synchronous crate may depend on
it.

- **Strict Async Isolation**: `tokio`, `tokio-tungstenite`, and `reqwest` MUST
  ONLY be added to `kurumi-feed`. Never introduce async runtime dependencies
  into `kurumi-core`, `kurumi-math`, `kurumi-ta`, `kurumi-pricing`,
  `kurumi-risk`, `kurumi-portfolio`, or `kurumi-backtest` (including their
  dev-dependencies). A default `kurumi` facade build MUST NOT pull a runtime.
- **Pure Rust TLS**: Configure `tokio-tungstenite` and `reqwest` with `rustls`
  (disabling default OpenSSL/native-tls) to ensure portable builds. Do not add
  an OpenSSL or `native-tls` feature path.
- **Untrusted Input Safety**: Network frames and binary ITCH buffers are
  untrusted inputs. Never use `.unwrap()`, `.expect()`, or unchecked slice
  indexing during parsing; validate lengths, bounds, and UTF-8 and return a
  typed `FeedError`. A malformed frame MUST NOT panic a Tokio worker.
- **Direct Decimal Decoding**: Decode exchange prices and quantities directly
  into `rust_decimal::Decimal` (e.g., ITCH fixed-point integer prices via
  `Decimal::new(val as i64, 4)` and JSON prices via string/decimal
  deserialization) without routing through `f64`. The only `f64` boundary is
  the `kurumi-ta` OHLCV input, after validation.
- **Offline Deterministic CI**: All default `cargo test` suites MUST run
  completely offline using local loopback mock WebSocket/HTTP servers and
  embedded binary fixtures. Mark live external endpoint tests with `#[ignore]`
  reading credentials strictly from environment variables.
- **Connector Contract**: A provider implements `MarketDataConnector` only: it
  publishes `MarketEvent`s, consumes `ConnectorCommand`s, and honours the
  shutdown signal. Backoff, heartbeat supervision, and automatic
  re-subscription belong to `ManagedConnector`; providers MUST NOT reimplement
  them.
- **Resilient Re-subscription**: The runtime owns the authoritative
  `BTreeSet<Subscription>`. After a transient failure it MUST reconnect with
  exponential backoff bounded by `max_backoff`, pass the complete active set to
  the new session, and stop when `max_retries` is exhausted. Shutdown MUST drop
  channels and tasks without leaking a provider task.
- **Hub Fan-Out**: `MarketDataHub` routes by provider name or venue
  deterministically (first registration wins for venue routing) and stores
  routing tables in `BTreeMap`. Consumers receive independent broadcast streams:
  a lagging consumer observes a `Lagged` count and MUST NOT block other
  consumers or the provider.
- **Bar Aggregation**: Align buckets from the exchange timestamp, keep prices,
  volume, and turnover in `Decimal`, and finalize a bar only when its bucket
  closes or an explicit flush is requested. Trades in an already-finalized
  bucket are dropped as late; ordering inside a bucket follows the exchange
  timestamp, not arrival order. Never present a partial bar as final.
- **Dependency Hygiene**: Declare every async dependency version once in the
  root `[workspace.dependencies]`; `kurumi-feed` inherits it and may only add
  feature flags. Keep `Cargo.lock` committed.

## 8. Coding & Testing Standards

- Annotate hot-path math functions with `#[inline]`.
- Document every public struct, trait, enum, function, and module with Rustdoc,
  including the governing mathematical formula, units, conventions (day count,
  compounding, annualization), and a runnable `# Examples` section where
  practical.
- Every financial formula or pricing model MUST include unit tests verified
  against standard textbook benchmarks (Hull, QuantLib, or a cited paper) using
  `approx::assert_relative_eq!` with an explicit `epsilon` or `max_relative`.
  Never compare floating-point results with `==`.
- Keep discrete money math in `Decimal`; do not round-trip currency amounts
  through `f64`. Use checked arithmetic where overflow is a real domain risk
  (notionals, quantities, accumulated cashflows).
- Tests live next to the code in `#[cfg(test)] mod tests`, or in `tests/` for
  integration tests. Name tests after the behavior and the benchmark they
  verify.
- Run formatting and all checks after completing any module, and fix every
  warning before moving on:

  ```sh
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  cargo doc --workspace --no-deps
  ```

- When changing facade features, also verify the feature matrix:
  `cargo check -p kurumi --no-default-features` and
  `cargo check -p kurumi --all-features`.
- New public APIs require Rustdoc and tests in the same change. Do not commit
  commented-out code or dead modules.
- Asynchronous tests in `kurumi-feed` use `#[tokio::test]` and the in-memory
  `MockReplayProvider`; they must not depend on live endpoints or wall-clock
  sleeps beyond a short bounded timeout.

## 9. Dependency Management

- Declare every external dependency version once in `[workspace.dependencies]`
  in the root `Cargo.toml`. Member manifests inherit it with
  `{ workspace = true }` and may add only feature flags.
- Member packages inherit shared metadata from `[workspace.package]` (version,
  edition, `rust-version`, license, repository); do not hard-code those fields.
- The workspace targets Rust 1.85+ and Edition 2024. Do not raise the MSRV or
  add a dependency that requires a newer toolchain without an explicit decision.
- Prefer established, well-maintained crates. Add a dependency only when it
  earns its place; small, well-understood numerical helpers belong in
  `kurumi-core` or `kurumi-math` instead.
- Keep `Cargo.lock` committed so workspace builds are reproducible.
- Async dependencies (`tokio`, `tokio-tungstenite`, `reqwest`, `futures-util`,
  `bytes`, `serde_json`, `tracing`) are declared once in the workspace and may
  be consumed only by `kurumi-feed`.

## 10. Definition of Done

A change is complete only when:

1. `cargo fmt --all -- --check` passes.
2. `cargo clippy --workspace --all-targets -- -D warnings` passes.
3. `cargo test --workspace` passes offline, including the new benchmark and
   feed tests.
4. `cargo doc --workspace --no-deps` builds without warnings.
5. Every new public item is documented with formula, units, and examples.
6. The crate layering and feature graph are unchanged, or the intentional change
   is explained in the PR.
