# pg-shell

A lightweight desktop Postgres query tool aiming for the feel of Azure Data Studio with the intellisense quality of Redgate SQL Prompt in SSMS.

> **Status — early WIP.** The architecture is locked in and a vertical slice is coming together. v1 honestly targets ~70% Redgate parity; see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full plan and honest scope notes.

## Why

Existing Postgres clients miss the spot Rodrigo (and probably you) actually want:

- **pgAdmin / DBeaver** — feature-rich but heavy and the editor isn't great.
- **Beekeeper / Tabularis** — light, but completion is shallow.
- **Azure Data Studio** — perfect weight, but Microsoft retired it and Postgres support was always a side feature.
- **pgMagic / Datagrip** — closer to Redgate-class, but commercial, big install, and not Postgres-first.

`pg-shell` is the tool we want: ADS-class lightness, Postgres-native, with a completion engine that knows your aliases, ranks by what you actually use, and ships snippets like `ssf` → `SELECT * FROM `.

## Stack

- **Backend:** Rust — `tauri` v2, `sqlx`, `pg_query` (libpg_query), `keyring`, `dashmap`.
- **Frontend:** Vite + React 19 + Monaco Editor + TanStack Virtual + Zustand.
- **Shell:** System WebView (WebView2 on Windows). No Electron, no bundled Chromium.

The Cargo workspace is split so the intellisense engine can be reused or tested in isolation:

```
pg-shell/
├── Cargo.toml              workspace root
├── package.json            Vite + React + Monaco frontend
├── src-tauri/              Tauri binary crate + IPC commands
├── crates/
│   ├── pg-core             sqlx pool manager, query exec, streaming, cancel
│   ├── pg-intellisense     tokenizer + partial-parse + context + ranker + snippets
│   ├── pg-schema-cache     pg_catalog introspection + DashMap + bincode snapshots
│   └── pg-profiles         connection profiles + OS keychain
└── src/                    React UI: editor, results grid, object tree
```

## Prerequisites

### Windows (primary target)

- **Rust stable** with the MSVC toolchain (`rustup install stable`)
- **Visual Studio Build Tools 2022** with the "Desktop development with C++" workload — needed by `cc` for the `pg_query` C build
- **Node 20+** and **pnpm 9+** (`corepack enable` is the easiest path)
- **WebView2** runtime (preinstalled on Windows 11)

### macOS / Linux

- Rust stable + standard C toolchain (`build-essential` / Xcode CLT)
- `webkit2gtk-4.1` on Linux (see [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/))
- Node 20+ and pnpm 9+

### Optional: `libpg_query`-enhanced intellisense

The intellisense engine ships with a token-based FROM-binding extractor that handles the vast majority of queries. For stricter alias resolution inside LATERAL joins, CTEs, and deeply-nested subqueries, enable the `libpg_query` feature. It uses Postgres's own parser via the `pg_query` crate, which requires LLVM:

```powershell
winget install LLVM.LLVM
# Reopen the shell so LIBCLANG_PATH resolves.
```

```sh
# macOS
brew install llvm
# Debian/Ubuntu
sudo apt install llvm-dev libclang-dev clang
```

Then in `src-tauri/Cargo.toml` add `features = ["libpg_query"]` to the `pg-intellisense` dependency:

```toml
pg-intellisense = { path = "../crates/pg-intellisense", features = ["libpg_query"] }
```

The engine picks the AST automatically when the buffer parses and falls back to the token walker when it doesn't, so this is a pure upgrade — no behavior change for simple queries.

## Develop

```sh
pnpm install
pnpm tauri dev
```

The first build takes a while (sqlx + tauri are not small). Subsequent rebuilds are fast.

## Release build

```sh
pnpm tauri build
```

Artifacts land in `src-tauri/target/release/bundle/`.

## What works today / what's next

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full sequenced roadmap. Headline items:

- [x] Workspace scaffold, Tauri boots, React shell
- [x] Connection profiles + OS keychain (passwords never on disk)
- [x] Test connection on new and saved profiles
- [x] Query execution with streaming results
- [x] Query cancellation
- [x] Schema introspection + object tree (Tree + Flat views)
- [x] Type-aware grid rendering (NULL, jsonb, bytea, timestamptz, numeric)
- [x] CSV / TSV / JSON export from the results grid
- [x] Monaco completion provider wired through Tauri IPC
- [x] Token-based + libpg_query AST partial-parse pipeline
- [x] Ranker + MRU persistence (SQLite-backed accept counts)
- [x] Builtin snippet library (`ssf`, `sf`, `ct`, `j`, `lj`, `cte`, …)
- [x] Right-click "Script as SELECT / INSERT" + "View Definition"
- [x] Function signature help
- [x] DDL-triggered schema cache refresh (object tree auto-updates after `CREATE`/`ALTER`/`DROP`)
- [ ] User-defined snippets
- [ ] Auto-alias on table insert (`FROM users` → `FROM users u`)
- [ ] Cross-CTE column binding, LATERAL join awareness
- [ ] Refactor-rename, semantic squigglies

## Contributing

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a PR — it covers local setup, the testing strategy (golden-file completion corpus + `testcontainers` for the schema cache), and the coding conventions we hold the line on.

If you're using Claude Code or another agent to work on the repo, [CLAUDE.md](CLAUDE.md) is the orientation file — it explains the architecture in agent-readable form and points at the right crates for each kind of change.

## License

MIT — see [LICENSE](LICENSE).
