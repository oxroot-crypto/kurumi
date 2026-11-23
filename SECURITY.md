# Security Policy

## Supported versions

The `main` branch is the only supported version. There are no tagged releases,
and the `kurumi` facade is not published on crates.io.

## Reporting a vulnerability

Do not open a public issue containing exploit details.

Use [GitHub's private vulnerability reporting](https://github.com/oxroot-crypto/kurumi/security)
from the repository's Security tab if it is enabled. If it is not enabled, open
a minimal issue stating that you have a security report and need a private
channel; do not include the reproduction or impact in that issue. A maintainer
will arrange private contact.

Once a private channel is available, include:

- the affected crate and commit;
- a minimal reproduction or malformed input;
- the observed and expected behavior;
- the potential impact and any suggested fix.

## Scope

Reports are especially relevant for:

- parsing untrusted WebSocket frames or Nasdaq ITCH buffers in `kurumi-feed`;
- panics, unbounded allocation, or denial of service on library paths;
- incorrect decimal accounting, ledger reservations, or order-state
  transitions in `kurumi-exec`;
- feature combinations that unexpectedly pull an async runtime or break the
  one-way crate layering;
- numerical results that violate the documented formulas or conventions.

## Disclosure

Please allow maintainers to investigate and prepare a fix before public
disclosure. Maintainers will coordinate a disclosure timeline with the
reporter when possible.
