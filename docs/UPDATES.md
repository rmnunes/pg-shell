# Auto-updates

pg-shell ships with the Tauri v2 updater plugin. Released builds check
`https://github.com/rmnunes/pg-shell/releases/latest/download/latest.json` on
startup and surface an "Update available" banner if a newer signed version is
published.

## One-time setup (before the first signed release)

The signing keypair is **not** in the repo; without it, the updater UI loads
but every check fails and is silently dismissed.

1. Generate a keypair locally (requires the Tauri CLI):

   ```sh
   pnpm tauri signer generate -w ~/.tauri/pg-shell.key
   ```

   You'll be prompted for a password. Keep both the file and the password —
   **lose them and you cannot ship updates to existing installs**, only to
   fresh installs (which won't carry the old pubkey).

2. Copy the public key it prints, and paste it into
   [src-tauri/tauri.conf.json](../src-tauri/tauri.conf.json) at
   `plugins.updater.pubkey`. Commit and push that change.

3. Add two repo secrets in GitHub
   (`Settings → Secrets and variables → Actions`):
   - `TAURI_SIGNING_PRIVATE_KEY` — the **contents** of `~/.tauri/pg-shell.key`
     (`cat ~/.tauri/pg-shell.key`, paste the whole base64 blob).
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` — the password you set in step 1.

4. Push a tag (`git tag v0.3.0 && git push --tags`). The release workflow now
   builds, signs, and uploads `latest.json` alongside the installers.

## How a release reaches users

1. CI builds bundles for Windows / macOS / Linux and signs each with the
   private key.
2. `tauri-action` uploads installers **and** `latest.json` (the signed update
   manifest) to the GitHub Release for that tag.
3. On startup, every running pg-shell fetches `latest.json` from the
   `/latest/download/` URL, compares versions, and surfaces the
   `UpdateBanner` if the remote version is newer.
4. "Update now" downloads the platform-appropriate bundle, verifies its
   detached signature against the pubkey baked into the app, and runs the
   installer. On Windows the installer is launched in passive mode (silent UI,
   progress bar only — see `windows.installMode: "passive"` in the config).
5. "Restart now" calls `plugin-process`'s `relaunch()`, exiting the current
   process so the freshly-installed binary takes over.

## Rotating the key

If the key leaks or is lost:

- **Lost (no leak):** generate a new keypair, swap the pubkey in
  `tauri.conf.json`, replace the secret. Old installs **cannot** update —
  they'll keep failing the signature check silently. Users must reinstall
  manually.
- **Leaked:** same procedure, plus invalidate the secret in the GitHub UI
  immediately. There's no built-in revocation; the only mitigation is to
  rotate and tell users to reinstall.

## Skipping updater UI in dev

`pnpm tauri dev` runs unsigned local builds — the updater plugin is still
present but the check naturally fails because the version in the manifest is
the latest. To force a check during development, edit `Cargo.toml` to bump
the package version below the latest release, rebuild, and the banner will
appear.
