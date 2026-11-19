# Kurumi

Kurumi is a high-performance, institutional-grade financial library written in
Rust (Edition 2024). It is organized as a Cargo workspace with strict, one-way
crate layering, so numerical primitives, pricing engines, risk analytics, and
backtesting tooling can be used together or independently.

## Workspace layout

| Crate | Responsibility |
| --- | --- |
| `kurumi-core` | Money, high-precision numerics, calendars, day-count conventions, fixed-capacity ring buffers, and the shared error type. |
| `kurumi-math` | Interpolation, root finding, random number generation, statistics, correlation and covariance estimation, regression, and linear algebra. |
| `kurumi-ta` | Candle structures plus O(1) streaming and vectorized batch technical indicators. |
| `kurumi-pricing` | Cashflows, yield curves, bonds, and option pricing engines. |
| `kurumi-risk` | VaR, CVaR, Greeks aggregation, Sharpe ratio, drawdowns, and benchmark-relative metrics. |
| `kurumi-portfolio` | Multi-asset panel alignment, return matrices, portfolio performance, risk decomposition, and rolling beta/correlation panels. |
| `kurumi-backtest` | Order-book matching simulation, slippage and fee models, and position tracking. |
| `kurumi` | Facade crate with feature-gated re-exports and the Python/WASM bindings. |

The facade enables the whole library by default. Individual layers can be
selected with Cargo features (`core`, `math`, `ta`, `pricing`, `risk`,
`portfolio`, `backtest`), and `full` enables them all.

## Building

Rust 1.85 or newer is required.

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

See [AGENTS.md](AGENTS.md) for the architecture invariants and contribution
standards.

## License

Released into the public domain under the [Unlicense](LICENSE).
