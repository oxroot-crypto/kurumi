# AGENTS.md

Kurumi is a pnpm-workspace TypeScript monorepo for a runtime-agnostic agent SDK plus a finance domain/stack. Use pnpm, not npm/yarn. Node **24.x or >=26.x** only (`.npmrc` sets `engine-strict=true`); packageManager is pnpm 12.14.0.

## Commands

```bash
pnpm install
pnpm lint && pnpm format:check && pnpm typecheck && pnpm test:coverage
pnpm clean && pnpm build        # build from scratch; do not trust stale dist
pnpm --filter @kurumi/agent typecheck
pnpm --filter @kurumi/agent test
pnpm --filter @kurumi/agent exec vitest run src/session.test.ts  # one test file
pnpm format                     # prettier write
```

- CI order is `lint -> format:check -> typecheck -> test:coverage -> clean -> build`.
- Do **not** use `pnpm test -- <path>`: the `--` makes Vitest ignore the path and run the whole package. Use `exec vitest run <path>`.
- Coverage thresholds are shared in `vitest.shared.ts` (85 statements/lines/functions, 75 branches).
- Tests and typecheck resolve `@kurumi/*` to workspace `src` (tsconfig `paths` + Vitest aliases), so no build is needed. Published `exports` point at `dist`, so examples and real consumers need `pnpm build` first.
- Filtered builds run `prebuild` -> `scripts/ensure-deps.mjs` to build missing dependency `dist` folders. When adding a workspace package, update `PACKAGE_DIRECTORIES` there and the dependents' `prebuild` args. After changing a dependency's source, `pnpm clean` before building so dependents do not consume stale `dist`.

## Workspace layout

- `packages/*`: `core`, `schema`, `config`, `log`, `agent`, `math`, `finance` libraries.
- `providers/*`: model adapters (`open-ai`, `anthropic`) and read-only market/fundamental data adapters (`nasdaq`, `binance`, `frankfurter`, `us-treasury`, `sec-edgar`).
- `tools/finance` publishes `@kurumi/finance-tools` — directory and package name differ.
- `examples/finance-agent` is the private live-verification app; `apps/kurumi-desktop` is an empty placeholder.
- Dependency direction: `core` is the base; `schema`/`config`/`log`/`agent`/`math` build on it. `finance` depends on core+math; data providers depend on core+finance+math; model adapters depend on agent+core; `finance-tools` depends on agent/finance/math/schema but **not** concrete providers.

## Hard architecture constraints

- Main entrypoints stay runtime-agnostic: no `node:` imports in core/schema/config/log/agent/math/finance or any provider adapter. Node-only code belongs under `packages/{core,config,log}/src/node` and is exported via `@kurumi/<pkg>/node`.
- No `console.*` in those packages (the log package's own sinks are the exception); use injected loggers or event streams.
- `architecture.test.ts` files enforce these boundaries. Extend them for new constraints; never delete or weaken them to make a change pass.
- Data providers are read-only: never add submit/cancel/execute-order or account-transfer methods. They accept `fetchImpl`/`timeoutMs`/`nowMs`; unit tests inject fakes and must not hit the network.
- All money/accounting uses `Decimal` (`bigint` + decimal scale), never floating point or `parseFloat`. Finance keeps transcendental math confined to `fixed-income.ts` (enforced by architecture test).

## Repo conventions

- Contract-first: `docs/spec/<pkg>.md` holds formal constraints (FC IDs) and a traceability matrix. For behavior/API changes update spec -> tests -> implementation, and keep matrix references current.
- Comments, TSDoc, test names, docs, and commit messages are Chinese; follow the target package's existing language and style rather than translating.
- Library relative imports use `.js` suffixes (`./errors.js`); only the example uses `.ts` imports. Use `import type` for type-only imports.
- TypeScript is stricter than stock `strict`: `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`, `verbatimModuleSyntax`, `erasableSyntaxOnly` (no `enum`/`namespace`/parameter properties). Use `as const` objects instead of enums.
- Shared dependency versions live in `pnpm-workspace.yaml` under `catalog:`; use `"catalog:"` rather than pinning duplicates. Add a changeset for published-package changes (`pnpm changeset`); releases use `pnpm release` from `main`.
- Tests are colocated `src/*.test.ts`; shared fixtures are `*.test-utils.ts`; property tests use fast-check. Do not lower coverage thresholds or remove architecture tests to get green.
- Live verification: copy `.env.example` to `.env`, set `KURUMI_API_KEY` and `KURUMI_SEC_USER_AGENT`, then run `pnpm --filter @kurumi/example-finance-agent verify`. It builds all packages, calls real APIs/DeepSeek, and writes the gitignored `report.md`; keep it out of unit/CI loops.
