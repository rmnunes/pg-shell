# Changelog

All notable changes to pg-shell. Releases are tagged `vX.Y.Z` and published on
the [Releases page](https://github.com/rmnunes/pg-shell/releases); installed
copies pick them up through the in-app updater.

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
