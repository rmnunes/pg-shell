# Contributing to pg-shell

Thanks for considering a contribution. This is a small project with a clear goal — Postgres desktop client with Redgate-class intellisense — so the bar for "does this fit?" is mostly: does it move us toward that goal without bloating the binary or the editor?

## Getting set up

1. Read [README.md](README.md) for prerequisites (Rust + MSVC on Windows, Node 20+, pnpm, optional LLVM for `pg_query` AST features).
2. Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — it's the single source of truth for what each crate does and why.
3. Clone, then:

   ```sh
   pnpm install
   pnpm tauri dev
   ```

4. To run the Rust test suite:

   ```sh
   cargo test --workspace
   ```

   Some tests in `pg-core` and `pg-schema-cache` use [`testcontainers`](https://docs.rs/testcontainers) to spin up `postgres:16`. Docker (or Podman) must be running. They're gated behind `--ignored` if you want to skip them:

   ```sh
   cargo test --workspace -- --skip postgres
   ```

## Where things live

| Concern | Crate / dir |
|---|---|
| sqlx pool, query execution, streaming, cancellation | [`crates/pg-core`](crates/pg-core) |
| Tokenizer, partial-parse, context detection, ranker, snippets | [`crates/pg-intellisense`](crates/pg-intellisense) |
| `pg_catalog` introspection, schema cache, persistence | [`crates/pg-schema-cache`](crates/pg-schema-cache) |
| Connection profile JSON + OS keychain via `keyring` | [`crates/pg-profiles`](crates/pg-profiles) |
| Tauri IPC commands and app state | [`src-tauri/src`](src-tauri/src) |
| React UI (editor, results grid, object tree) | [`src`](src) |

If you're not sure which crate a change belongs in, the rule of thumb: anything pure Postgres (no Tauri types, no UI assumptions) goes in a `crates/*` crate; anything that needs `tauri::AppHandle` lives in `src-tauri/src/commands/`.

## Coding conventions

- **Rust:** `cargo fmt` and `cargo clippy --workspace --all-targets -- -D warnings` must pass. CI enforces both.
- **TypeScript:** `tsc -b` clean; we don't have a separate lint step yet.
- **Errors:** All Tauri commands return `Result<T, AppError>` where `AppError: Serialize`. Don't leak `anyhow::Error` across the IPC boundary — convert at the command layer.
- **Logging:** `tracing` only, no `println!` in library code. The Tauri main initializes the subscriber.
- **Schema queries:** prefer `pg_catalog` over `information_schema` — it's faster, more complete, and covers things like `relkind` properly.
- **No `unsafe`** in the workspace crates. If you genuinely need it, raise an issue first.
- **No async in the tokenizer / context / ranker.** Intellisense is hot path; keep it sync and CPU-bound.

## Testing strategy

- **Intellisense changes** must come with at least one new entry in the golden-file corpus under `crates/pg-intellisense/tests/corpus/` — `{sql, cursor_offset}` → expected ranked completion kinds. Don't move existing goldens without explaining why in the PR description.
- **Schema cache changes** need an integration test against a real Postgres container. Catalog shapes drift across major versions; we test against `postgres:16` as the floor.
- **UI changes** that touch the results grid or completion provider should include a manual repro in the PR description (paste a SQL snippet + screenshot or terminal log of what should happen).

## Pull request flow

1. Fork, branch off `main`.
2. Keep PRs small. A new completion context, a new `pg_catalog` query, a UI bug fix — those are the right unit. "Add ranker + MRU + snippets" should be three PRs.
3. PR titles: imperative, present tense, no ticket prefix. `add CamelHump prefix scoring to ranker`, not `Added CamelHump scoring (#42)`.
4. Fill in the PR template — especially the "what I tested" section.
5. CI must be green before review.

## Releases

Releases are tag-triggered. Pushing a `vX.Y.Z` tag to `main` kicks off `.github/workflows/release.yml`, which uses [tauri-action](https://github.com/tauri-apps/tauri-action) to build on Windows / macOS / Linux runners and attach the platform-native bundles to a GitHub Release:

- Windows: `.msi` + `.exe` (NSIS)
- macOS: `.dmg` + `.app.tar.gz`
- Linux: `.deb` + `.AppImage` + `.rpm`

Users download from the [Releases page](https://github.com/rmnunes/pg-shell/releases).

### Cutting a release

Versions live in five places that have to stay in lockstep: `package.json`, `src-tauri/tauri.conf.json`, and the `[package].version` of each Cargo manifest (`src-tauri/Cargo.toml` + the four crates under `crates/`). The `pnpm release` script handles all of them plus regenerating `Cargo.lock`:

```sh
pnpm release patch       # 0.1.0 -> 0.1.1
pnpm release minor       # 0.1.0 -> 0.2.0
pnpm release major       # 0.1.0 -> 1.0.0
pnpm release 1.2.3       # set explicitly
```

After the script finishes:

```sh
git diff                                  # sanity check
git commit -am "chore: release v0.1.1"
git tag v0.1.1
git push origin main v0.1.1
```

Push the tag by name: `git push --follow-tags` only sends *annotated* tags, and `git tag vX.Y.Z` creates a lightweight one, so the release workflow would never fire.

The release workflow takes ~10–15 minutes per OS.

### Versioning rules of thumb

We're pre-1.0, so:

- **Breaking change** (IPC contract, profile JSON shape, persistence format): minor bump.
- **New feature, bug fix, performance improvement**: patch bump.
- Once we hit 1.0, switch to standard SemVer.

## Reporting bugs

Use the GitHub issue templates. For intellisense bugs, **please include the exact SQL buffer and cursor position** — without it we can't reproduce. A `cursor: 47` byte offset or a `|` marker in the SQL works equally well.

## Scope boundaries (what we're saying no to, for now)

- Non-Postgres database support. The intellisense relies on `pg_catalog` and `pg_query`; abstracting over MySQL/SQLite is a different project.
- Web / hosted UI. Tauri-only by design — startup time and binary size are features.
- Cross-CTE column binding, function signature help, refactor-rename, semantic squigglies. These are real, but they're v2 work; v1 is about getting the fundamentals to feel Redgate-class.

If you want to attempt one of those anyway, open a discussion first — we'll talk it through.
