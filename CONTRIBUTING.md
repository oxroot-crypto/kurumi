# Contributing to Kurumi

Thanks for taking the time to improve Kurumi. This project follows the
architecture invariants and coding standards in [AGENTS.md](AGENTS.md); read
that file before changing crate boundaries, numerical conventions, or public
APIs.

## Before you start

- Search existing issues and pull requests before opening a new one.
- For bugs, include the affected crate, a minimal reproduction, and the
  expected behavior.
- For security issues, follow [SECURITY.md](SECURITY.md) instead of opening a
  public issue with exploit details.

## Development setup

Kurumi requires Rust 1.85 or newer.

```sh
git clone https://github.com/oxroot-crypto/kurumi.git
cd kurumi
cargo build --workspace
```

## Checks

Run the full suite before opening a pull request:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo doc --workspace --no-deps --locked
cargo check -p kurumi --no-default-features --locked
cargo check -p kurumi --all-features --locked
cargo build -p kurumi --examples --all-features --locked
```

CI also checks the declared MSRV with `cargo +1.85.0 check --workspace
--all-features --locked`.

## Code standards

- Library code must not panic: use `get`, `checked_*`, and `try_*` instead of
  `unwrap`, `expect`, or slice indexing.
- Use `rust_decimal::Decimal` for money, quantities, and cashflows. Use `f64`
  for continuous models, statistics, and indicators.
- Validate `NaN` and infinite values at public boundaries and return the
  project error type.
- Document every public item with Rustdoc, including units, conventions, and a
  runnable example where practical.
- Add unit tests with textbook or cited benchmarks for financial formulas.
- Keep the one-way crate layering. Do not add an async dependency outside
  `kurumi-feed` and the opt-in `kurumi-exec` `live` feature.
- When changing facade features, verify both feature-matrix commands above.

## Pull requests

Keep each pull request focused on one logical change. Explain any change to
crate layering, feature implications, or public API behavior, and update the
README and CHANGELOG for user-visible changes.
