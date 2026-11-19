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
| `kurumi-math` | Numerical methods: interpolation, root finding, random number generation, statistics, linear algebra. |
| `kurumi-ta` | Time series and technical indicators: candle structures, O(1) streaming incremental indicators, vectorized batch indicators. |
| `kurumi-pricing` | Asset and derivative pricing: cashflows, yield curves, bonds, option pricing engines. |
| `kurumi-risk` | Risk and performance analytics: VaR, CVaR, Greeks aggregation, Sharpe ratio and drawdowns. |
| `kurumi-backtest` | Strategy backtesting: order-book matching simulation, slippage and fee models, account/position tracking. |
| `kurumi` | Top-level facade: re-exports the member crates behind Cargo features and hosts the Python/WASM bindings. |

### Crate layering

Dependencies flow in one direction only:

```text
kurumi-core  <  kurumi-math  <  kurumi-ta / kurumi-pricing  <  kurumi-risk  <  kurumi-backtest  <  kurumi
```

A crate may depend only on crates that appear earlier in this list. `kurumi-ta`
and `kurumi-pricing` are peers: neither may depend on the other. `kurumi-core`
has no workspace dependencies. The `kurumi` facade depends on every member
crate, and no member may depend on the facade.

Crate boundaries are part of the public architecture: do not merge crates,
introduce a new crate, or add an edge that reverses the layering without an
explicit architectural decision recorded in the PR.

## 3. Architectural Invariants

- **Workspace structure**: maintain the seven crates and the one-way layering
  above. No dependency cycles.
- **Dual numerical precision**:
  - Use `rust_decimal::Decimal` for currency amounts, ledger accounting, order
    prices, and discrete cashflows.
  - Use `f64` for continuous pricing models (Black-Scholes, Monte Carlo),
    root-finding, and statistical indicators.
  - Reject `NaN` and infinite values at every public API boundary using typed
    errors; never let them silently propagate.
- **Zero-panic policy**: never use `.unwrap()`, `.expect()`, or array/slice
  indexing `[]` in library code. Every fallible operation must return
  `Result<T, KurumiError>`. Use `get`, `checked_*`, and `try_*` APIs instead.
  Assertions are appropriate in tests, not on library paths. Member crates deny
  `clippy::unwrap_used`, `clippy::expect_used`, and
  `clippy::indexing_slicing` outside `cfg(test)`; any scoped `#[allow(...)]`
  needs a written justification.
- **Borrow checker simplicity**: prefer `Copy` value types (`Price`, `Rate`,
  `Date`, `Currency`) over complex reference lifetimes, and avoid
  self-referential structs.
- **Error discipline**: `KurumiError` (in `kurumi-core`) is the single public
  error type. Add variants with a descriptive `#[error("...")]` message, keep
  the originating context, and do not leak dependency error types through
  public APIs.
- **Safety**: library code is safe Rust. `unsafe_code` is denied workspace-wide;
  any unavoidable exception needs a scoped `#[allow(unsafe_code)]`, a `// SAFETY:`
  comment covering every invariant, and review justification.
- **Facade features**: `kurumi` exposes each member behind an additive feature
  (`core`, `math`, `ta`, `pricing`, `risk`, `backtest`). Features are additive:
  enabling one must not break another, and lower-layer features are implied by
  higher-layer ones. Default features enable the whole library. Keep the
  feature graph in sync with the crate layering.
- **Bindings**: Python/WASM bindings live in `kurumi` behind dedicated opt-in
  features and must be thin adapters over the Rust API. Business logic never
  lives in binding code.

## 4. Coding & Testing Standards

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

## 5. Dependency Management

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

## 6. Definition of Done

A change is complete only when:

1. `cargo fmt --all -- --check` passes.
2. `cargo clippy --workspace --all-targets -- -D warnings` passes.
3. `cargo test --workspace` passes, including the new benchmark tests.
4. `cargo doc --workspace --no-deps` builds without warnings.
5. Every new public item is documented with formula, units, and examples.
6. The crate layering and feature graph are unchanged, or the intentional change
   is explained in the PR.
