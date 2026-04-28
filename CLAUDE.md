# CLAUDE.md

Orientation for Claude Code (and other coding agents) working in this repo. Humans should read [README.md](README.md) and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) instead — this file is tuned for LLM context, not prose.

## What this project is

`pg-shell` is a desktop Postgres query tool. Two non-negotiable goals:

1. **Lightweight like Azure Data Studio.** Tauri v2 + system WebView. No Electron. Release binary target ≤ 20 MB.
2. **Intellisense like Redgate SQL Prompt.** Snippets (`ssf` → `SELECT * FROM `), table-aware `alias.` completion, MRU-aware ranking. v1 honestly aims for ~70% Redgate parity.

The architecture plan in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) is authoritative. If a change conflicts with it, update the plan in the same PR — don't silently diverge.

## Codebase shape

```
src-tauri/                    Tauri binary + IPC commands; keep glue code here, real logic in crates
crates/
  pg-core/                    sqlx pool per profile, streaming exec, cancellation
  pg-intellisense/            tokenize → partial-parse → context → ranker → snippets
  pg-schema-cache/            pg_catalog introspection, DashMap, bincode persistence
  pg-profiles/                profile JSON + OS keychain (`keyring` crate)
src/                          React UI (Vite + Monaco + TanStack Virtual + Zustand)
  editor/                     Monaco wiring + completionProvider.ts (calls Tauri IPC)
  results/                    virtualized grid
  tree/                       object explorer
```

### The intellisense engine — three-tier parser pipeline

This is the headline subsystem. It exists because the buffer is almost always mid-edit and `libpg_query` is all-or-nothing:

1. **Tokenize always.** Hand-written PG tokenizer in [crates/pg-intellisense/src/tokenize.rs](crates/pg-intellisense/src/tokenize.rs). Never fails. This is the ground truth for cursor context.
2. **Full parse when possible.** `pg_query::parse()` on the whole buffer; cache the protobuf AST keyed by content hash.
3. **Partial-parse recovery.** Split on top-level `;` via tokens; re-parse the statement containing the cursor. If it fails, truncate tokens back from cursor and retry until `pg_query` accepts, or bail to pure token-walk.

**Context detector** ([crates/pg-intellisense/src/context.rs](crates/pg-intellisense/src/context.rs)) walks tokens backward from cursor, paren-depth aware, classifying into `Keyword | Schema | Table | Column | Function | ColumnOfAlias | SchemaOfTable | Snippet`.

**Alias resolution** ([crates/pg-intellisense/src/aliases.rs](crates/pg-intellisense/src/aliases.rs)) walks `SelectStmt.fromClause` proto nodes (`RangeVar`, `JoinExpr`) when the AST is good; falls back to a conservative token scan of `FROM ... (WHERE|GROUP|ORDER|LIMIT|HAVING|;|EOF)` when not.

**Ranker** ([crates/pg-intellisense/src/ranker.rs](crates/pg-intellisense/src/ranker.rs)):
```
score = context_weight(kind)            // Table=100 in FROM ctx, etc.
      + prefix_score(prefix, name)      // exact=200, ci-prefix=150,
                                        // CamelHump=120, snake-initials=110,
                                        // substring=80, fuzzy=40
      + log2(1 + mru_count_30d) * 10
      + recency_bonus
```

## Hard rules

- **Don't depend on Supabase's `postgres-language-server`.** Use the `pg_query` crate (libpg_query) directly. The architecture plan calls this out explicitly.
- **No `unsafe` in workspace crates.**
- **Intellisense hot path is sync.** No `async` in tokenize / context / ranker. The IPC boundary in `src-tauri/src/commands/completion.rs` is the only async wrapper.
- **`AppError`, not `anyhow::Error`, across IPC.** Tauri commands serialize their errors; don't leak `anyhow` types to the frontend.
- **`pg_catalog` over `information_schema`** for introspection. It's faster and exposes `relkind` properly.
- **Passwords live in the OS keychain** via the `keyring` crate (service `"pg-shell"`, username = profile id). Never in JSON or logs.
- **No `println!` in library code.** Use `tracing`.

## Common tasks — where to start

| Task | Start here |
|---|---|
| New completion context (e.g. inside `WINDOW` clause) | [crates/pg-intellisense/src/context.rs](crates/pg-intellisense/src/context.rs) + add a corpus entry under `crates/pg-intellisense/tests/corpus/` |
| New `pg_catalog` query (e.g. publications, materialized view defs) | [crates/pg-schema-cache/src/introspect.rs](crates/pg-schema-cache/src/introspect.rs) |
| New IPC command | [src-tauri/src/commands/](src-tauri/src/commands/) — register in `main.rs` `invoke_handler` |
| Results grid behavior | [src/results/](src/results/) — uses TanStack Virtual, fixed 24px rows |
| Connection lifecycle | [crates/pg-core/src/pool.rs](crates/pg-core/src/pool.rs) and [crates/pg-profiles/src/](crates/pg-profiles/src/) |
| Snippets library | [crates/pg-intellisense/src/snippets.rs](crates/pg-intellisense/src/snippets.rs) |

## Verification before declaring done

- `cargo fmt --all` clean
- `cargo clippy --workspace --all-targets -- -D warnings` clean
- `cargo test --workspace` green
- `pnpm build` clean (TypeScript is `tsc -b` strict)
- For UI changes: `pnpm tauri dev`, drive the feature manually, paste the repro in the PR description

## What's deferred — don't accidentally implement these

These are real items but they're v2; please flag if a request slides into one rather than silently doing it:

- Cross-CTE column binding
- Function signature help / parameter hints
- Refactor-rename
- Semantic squigglies
- Dollar-quoted PL/pgSQL body completion
- Auto-alias on table insert (`FROM users` → `FROM users u`)
- Non-Postgres database support

## When in doubt

- Ask before you broaden scope. A bug fix doesn't need surrounding refactors.
- Ask before adding a new dependency. The Cargo workspace dependencies live in the root `Cargo.toml`; check there first.
- The plan in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) has an "Honest scope" section. v1 is about fundamentals, not parity.
