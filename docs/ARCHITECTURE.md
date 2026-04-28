# pg-shell — architecture

> This is the design doc the project is being built against. It captures the decisions and rationale that aren't visible from reading the code alone. Treat it as authoritative — if a code change conflicts with this plan, update the plan in the same PR.

## Context

A desktop Postgres query tool that feels like Azure Data Studio (lightweight, fast) with intellisense quality on par with **Redgate SQL Prompt** in SSMS (e.g. `ssf` → `SELECT * FROM `, table-aware column completion after `alias.`, context-ranked suggestions). Existing tools (pgAdmin, DBeaver, Beekeeper, Tabularis, pgMagic, DataGrip) all miss at least one of those two axes — `pg-shell` is the attempt to hit both.

### Locked-in decisions
- **Stack:** Rust + **Tauri v2** + **Monaco Editor** (system webview, no Electron). Chosen over egui because Monaco delivers the ADS-class editor for free; chosen over Python/PySide because binary size and startup matter.
- **Autocomplete engine:** built in-house in Rust around **`pg_query` crate** (wraps libpg_query — Postgres's own C parser, vendor-neutral). We do *not* depend on Supabase's postgres-language-server, but study its API shape.
- **V1 scope (all four pillars):** (1) multi-server connection manager, (2) Monaco editor with intellisense, (3) virtualized results grid + export, (4) object explorer tree.
- **Platform focus:** Windows 11 primary.

## Approach

### Repository layout (Cargo workspace + Vite frontend)

```
c:\dev\pg-shell\
├── Cargo.toml                # workspace root
├── package.json              # pnpm + Vite + React + Monaco
├── src-tauri\                # Tauri binary crate + IPC command layer
│   └── src\commands\         # connections.rs query.rs completion.rs schema.rs
├── crates\
│   ├── pg-core\              # sqlx pool mgr, query exec, streaming, cancel
│   ├── pg-intellisense\      # tokenizer + partial-parse + context + ranker + snippets + MRU
│   ├── pg-schema-cache\      # introspection, DashMap store, bincode persistence
│   └── pg-profiles\          # profile JSON + OS keychain via `keyring`
└── src\                      # React UI: editor, results grid, object tree, shell
```

### Intellisense engine (the headline feature — must feel Redgate-like)

Three-tier parser pipeline, because the buffer is almost always mid-edit and libpg_query is all-or-nothing:

1. **Tokenize always** — hand-written PG tokenizer in `pg-intellisense::tokenize` producing `Vec<Token>`. Handles quoted idents, dollar-quoted strings, comments. Never fails. This is the ground truth for cursor context.
2. **Full parse when possible** — feed the whole buffer to `pg_query::parse()`; cache the protobuf AST keyed by content hash.
3. **Partial-parse recovery** — split on top-level `;` via the token stream; re-parse the statement containing the cursor. If it fails, truncate tokens back from cursor and retry until `pg_query` accepts, or bail to pure token-walk.

**Context detector** walks tokens backward from the cursor (paren-depth aware), classifying into: `Keyword`, `Schema`, `Table/View`, `Column`, `Function`, `Column-of-alias`, `Schema-of-table`, `Snippet`. Disambiguating `<ident>.` checks alias bindings first, then schema names.

**Alias resolution** — when partial parse succeeds, walk `SelectStmt.fromClause` proto nodes (`RangeVar`, `JoinExpr`) to collect `FromBinding { schema, table, alias }`. When it fails, token-scan `FROM ... (WHERE|GROUP|ORDER|LIMIT|HAVING|;|EOF)` for `ident (AS)? alias` pairs. Lossy but adequate.

**Ranker** — composite score:
```
score = context_weight(kind)                  // Table=100 in FROM ctx, etc.
      + prefix_score(prefix, name)            // exact=200, ci-prefix=150,
                                              //   CamelHump=120, snake-initials=110,
                                              //   substring=80, fuzzy=40
      + log2(1 + mru_count_30d) * 10          // learned from user's accepts
      + recency_bonus                         // recently-created objects
```
MRU persisted to `%APPDATA%\pg-shell\mru.sqlite` via `rusqlite`, keyed `(profile_id, context_kind, identifier)`.

**Snippets** — builtin library (`ssf`, `sf`, `ct`, `ctas`, `j`, `lj`, `ij`, `cte`, `ins`, `upd`, `del`, `win`, `case`) emitted as Monaco completions with `InsertTextRules.InsertAsSnippet` and `${1:placeholder}` syntax. Offered only at statement-initial position to avoid noise. User snippets loaded from `%APPDATA%\pg-shell\snippets.json`.

**Auto-qualification** — on column accept in multi-binding FROM, backend returns `insertText = "alias.column"` (single-binding uses bare name). Auto-alias-on-table-insert (Redgate's `FROM users u` flourish) deferred to v1.1.

**Honest scope note:** v1 will feel ~70% of Redgate. 90% is a v2 milestone — Redgate has 15 years of edge-case handling in SQL Prompt. v1 defers: cross-CTE column binding, LATERAL joins, function signature-help / parameter hints, refactor-rename, semantic squigglies, dollar-quoted PL/pgSQL body completion.

### Schema cache

Introspect via **`pg_catalog`** (faster and more complete than `information_schema`):
- Schemas: `pg_namespace` filtered `NOT IN ('pg_toast','pg_catalog','information_schema') AND nspname NOT LIKE 'pg_temp_%'`
- Tables/views: `pg_class JOIN pg_namespace` on `relkind IN ('r','v','m','p','f')`
- Columns: `pg_attribute LEFT JOIN pg_attrdef`, with `format_type(atttypid, atttypmod)`
- Functions: `pg_proc` with `pg_get_function_identity_arguments` / `pg_get_function_result`
- Indexes: `pg_index JOIN pg_class` with `pg_get_indexdef`
- Foreign keys: `pg_constraint WHERE contype='f'` (for future Script-as-JOIN)

Storage: `DashMap<ProfileId, Arc<RwLock<DatabaseCatalog>>>` in-memory; `bincode` snapshots to `%APPDATA%\pg-shell\cache\<profile_id>.bin` for warm-cache on startup. Refresh triggers: on connect (full), on DDL detected in executed queries (inspect `pg_query` AST for `CreateStmt`/`AlterTableStmt`/`DropStmt`), manual refresh button.

### Connection manager

- Profiles in `%APPDATA%\pg-shell\profiles.json` — `{id, name, host, port, database, user, ssl_mode, app_name, group}`.
- Passwords **only** in OS keychain via `keyring` crate (service `"pg-shell"`, username = profile_id). Never in JSON.
- `sqlx::PgPool` per profile, lazy-created, `max_connections=5` default. Held in `ConnectionManager: DashMap<ProfileId, PgPool>`.
- Each query tab binds to one profile; status bar shows server/db/user/latency.
- Cancellation: `PgConnection::cancel_query` (Postgres CancelRequest protocol); fallback `SELECT pg_cancel_backend($pid)` via sibling connection.

### Results grid

- Streaming: `sqlx::query(...).fetch(&pool)` stream, batch 500 rows, emit Tauri event `query://<id>/rows`; final `query://<id>/done` with summary. Frontend appends to ring buffer.
- Virtualization: **TanStack Virtual** (row + column), fixed 24px row height, auto-measured + user-resizable columns. Handles 1M rows.
- Type rendering (dispatch on `PgTypeOid`): NULL dim-italic, bytea hex preview, jsonb collapsed with click-to-expand tree, arrays as `{a,b,c}`, timestamptz ISO-8601 with tz badge, numeric right-aligned as *string* (never f64), uuid monospace. Unknown types fall back to OID-named text.
- Export: CSV (RFC 4180 compliant), TSV for clipboard, JSON (typed). Streams from backend store, not DOM.

### Object explorer

- Lazy-load tree: Servers → Databases → Schemas → (Tables | Views | MatViews | Functions | Sequences) → object → Columns/Indexes/Constraints. Each expand calls `schema_browse(profile_id, path)`; warm cache returns instantly.
- Top filter box (substring), deeper server search on Enter.
- Right-click: Script as SELECT (`SELECT c1,c2,... FROM s.t LIMIT 100`), Script as INSERT (typed placeholders), View Definition (`pg_get_viewdef`/`pg_get_functiondef`), Refresh, Drop (modal confirm showing exact DROP stmt — never auto-execute).

### Tauri IPC command surface

All `async fn -> Result<T, AppError>` where `AppError: Serialize`:
- Connections: `connections_list`, `connection_create/update/delete`, `connection_test`, `connection_connect/disconnect`
- Queries: `query_execute(profile_id, sql, query_id)` (streams via events), `query_cancel(query_id)`
- Completion: `completion_get(profile_id, doc, cursor_offset)`, `completion_record_accept(profile_id, item_id)`
- Schema: `schema_browse(profile_id, path)`, `schema_refresh(profile_id, path?)`
- Scripting: `script_as_select`, `script_as_insert`, `object_definition`
- Settings: `snippets_list/upsert`, `settings_get/set`

## Critical files

- [Cargo.toml](Cargo.toml) — workspace root (create)
- [src-tauri/src/main.rs](src-tauri/src/main.rs) — Tauri builder, `.manage(AppState)`
- [src-tauri/src/commands/completion.rs](src-tauri/src/commands/completion.rs) — IPC glue to intellisense engine
- [crates/pg-intellisense/src/partial_parse.rs](crates/pg-intellisense/src/partial_parse.rs) — three-tier parser pipeline
- [crates/pg-intellisense/src/context.rs](crates/pg-intellisense/src/context.rs) — backward-walking context detector
- [crates/pg-intellisense/src/ranker.rs](crates/pg-intellisense/src/ranker.rs) — scoring function
- [crates/pg-schema-cache/src/introspect.rs](crates/pg-schema-cache/src/introspect.rs) — pg_catalog queries
- [crates/pg-core/src/pool.rs](crates/pg-core/src/pool.rs) — sqlx pool lifecycle per profile
- [src/editor/completionProvider.ts](src/editor/completionProvider.ts) — Monaco `CompletionItemProvider` calling Tauri

## Sequencing — one-week vertical slice

| Day | Deliverable |
|---|---|
| 1–2 | Workspace scaffold; Tauri boots; React shell; profiles CRUD + keyring; connect/disconnect |
| 3 | `query_execute` with row streaming; TanStack grid renders typed rows |
| 4 | Schema introspection (tables+columns+schemas); DashMap cache; object tree lazy expand |
| 5 | Monaco CompletionItemProvider wired; context detector for FROM/SELECT/`alias.`; alpha-sorted (no ranker yet) |

**End-of-week demo:** connect → browse tree → run query → basic completions fire. Credible MVP.

Weeks 2–3: full context set, ranker + MRU, snippets, type-aware grid, export, cancellation, partial-parse recovery.

## Risks

1. **Partial-parse recovery is O(tokens) worst case per keystroke.** Mitigate with 150ms debounce + prefix-hash caching of recent results.
2. **Alias resolution under broken SQL.** When parse fails and we token-walk, ambiguity yields junk. Mitigation: conservative walker returns "all columns of all FROM-mentioned tables" rather than wrong bindings.
3. **`pg_query` crate builds on Windows** need MSVC + `cc` crate. Document VS Build Tools as prerequisite in README; CI must run `windows-latest`.
4. **Monaco async completion + Tauri IPC latency** on WebView2. Measure early; may need JS-side pre-filter against last result set.
5. **keyring crate on Windows Credential Manager** has ~2.5KB/secret limit — fine for passwords.
6. **Redgate-parity expectations.** Be explicit with Rodrigo that v1 targets ~70% fidelity; set the right mental model now.

## Verification

**Unit tests**
- `pg-intellisense`: golden-file corpus of ~100 `{sql, cursor_offset}` → expected `Vec<CompletionKind>` cases. Assert context classification, alias resolution, ranker ordering.

**Integration tests**
- `pg-core` + `pg-schema-cache`: `testcontainers` spinning `postgres:16`; run all introspection queries, assert catalog shape; execute a simple query + cancel it.

**End-to-end smoke**
1. `pnpm install && pnpm tauri dev`
2. Create connection profile to local `localhost:5432` (password in keychain).
3. Connect; object tree shows schemas within 500ms of warm cache.
4. In editor type `SELECT * FROM pg_tables LIMIT 10;` — press F5 — 10 rows stream into virtualized grid; numeric columns right-aligned; export CSV round-trips through Excel.
5. In a new tab type `SELECT  FROM pg_catalog.pg_class c WHERE c.` — at the trailing `.`, completion popup lists `c`'s columns sourced from cache, ranked.
6. Type `ssf<Tab>` at BOF → expands to `SELECT * FROM `.
7. Execute `CREATE TABLE foo_test (id int);` — within 1s, `foo_test` appears in object tree (DDL-triggered refresh).
8. Start a 30-second query (`SELECT pg_sleep(30);`), press cancel — query terminates within 2s.

**Release build**
- `pnpm tauri build` → MSI in `src-tauri/target/release/bundle/msi/`. Open, install, launch; repeat smoke 1–8. Verify binary size ≤ 20 MB.
