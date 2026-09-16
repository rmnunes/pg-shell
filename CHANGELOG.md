# Changelog

All notable changes to pg-shell. Releases are tagged `vX.Y.Z` and published on
the [Releases page](https://github.com/rmnunes/pg-shell/releases); installed
copies pick them up through the in-app updater.

## v0.5.0 — 2026-09-16

### Fixed

- **The window no longer goes blank when a batch returns more than one result
  set.** Columns were announced once, on the first row-producing statement, and
  every later statement's rows were streamed under that same header. A script
  ending in SELECTs of different widths therefore handed 4-cell rows to a
  3-column grid; the renderer threw on the missing column and, with no error
  boundary anywhere, React unmounted the entire app. Nothing was wrong with the
  query — the results were simply unrenderable.
- Each result-set-producing statement now gets its own columns and its own
  rows. When a batch produces several, a **Result 1 / Result 2 / …** strip
  appears above the grid; hovering a chip shows that set's column names and row
  count. Batches with a single result set look exactly as before.
- CSV, TSV and JSON export act on the result set on screen rather than a
  flattened concatenation of every SELECT in the batch.
- The grid skips cells that have no matching column instead of throwing, so a
  shape mismatch can never take the window down again.

### Changed

- `query:start` and `query:rows` carry a `result_index`. Anything consuming
  those events directly needs to key rows by it rather than assume one grid per
  run.

## v0.4.1 — 2026-09-16

### Fixed

- Scripts written for psql no longer fail with `syntax error at or near "\"`.
  Backslash commands are psql's own, not SQL, so the server rejected them and
  pointed at a line that looked perfectly valid. Commands that only shape
  psql's output — `\echo`, `\pset`, `\timing`, `\x`, `\a`, `\t`, `\f`, `\C`,
  `\H`, `\h`, `\qecho`, `\warn` — are now dropped and the SQL runs unchanged.
- Anything that would change *what the script does* is still refused, but by
  name and line: "line 62: psql meta-command \gset is not supported…". That
  covers `\set`, `\gset`, `\i`, `\ir`, `\copy`, `\gexec`, `\watch`, `\d*` and
  friends. `\c` is deliberately in that group — silently dropping a `\connect`
  would run the rest of the script against a different database.
- Backslashes inside string literals, dollar-quoted bodies and block comments
  are left alone, including Windows paths like `'C:\dev\pg-shell'`. Stripped
  lines become blank rather than vanishing, so line numbers in later server
  errors still line up with the editor.

## v0.4.0 — 2026-09-04

### Added

- **Microsoft Entra MFA authentication** for Azure Database for PostgreSQL.
  Pick *Microsoft Entra MFA* in the connection dialog and sign in through your
  browser (MFA and Conditional Access included). The access token is used as
  the database password and refreshed silently while you work; only the
  refresh token is cached, in the OS keychain. *Sign out* in the profile
  editor forgets it.
- Leave **User** blank on an Entra profile to connect as the account you sign
  in with, or enter an Entra group's display name to connect as that group's
  role. Optional **Tenant** and **Client ID** fields cover guest tenants and
  organisations that register their own public client.
- Clearer error when Azure rejects a sign-in: the dialog explains which role
  the server was asked for and when to use a UPN versus a group name, instead
  of the server's misleading "password authentication failed".

### Internal

- New `pg-entra` crate (OAuth 2.0 authorization-code + PKCE on a loopback
  redirect, token refresh, session cache). `pg-core` pools now take a
  `Credential` and rotate token passwords ahead of expiry via
  `Pool::set_connect_options`.
- Profiles gain `auth_method` and optional `entra {tenant, client_id}`;
  existing `profiles.json` files load unchanged.

## v0.3.0 — 2026-05-23

### Added

- In-app auto-updater: checks for signed releases on startup, downloads and
  installs them, and relaunches.

### Fixed

- Multi-statement queries run over the simple protocol.
- Results pane is resizable.
- Query tabs show the target server.

## v0.2.0 — 2026-04-28

### Added

- Test a connection before saving it.
- Schema cache refreshes automatically after `CREATE` / `ALTER` / `DROP`.

## v0.1.0 — 2026-04-28

Initial public release: connection profiles with OS-keychain passwords,
streaming query execution with cancellation, object explorer, type-aware
results grid with CSV/TSV/JSON export, and the Redgate-style intellisense
engine (snippets, alias-aware completion, MRU ranking, signature help).
