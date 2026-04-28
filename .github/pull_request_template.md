## Summary

<!-- One or two sentences. What does this change and why. -->

## What I changed

-
-

## What I tested

<!-- Imperative checklist of what you actually ran. Tick what passed. -->

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `pnpm build`
- [ ] Manual repro in `pnpm tauri dev` (if UI / completion / results-grid touched)

## For intellisense changes

- [ ] Added at least one new entry to `crates/pg-intellisense/tests/corpus/`
- [ ] If existing goldens moved, explained why below

## Notes

<!-- Anything reviewers should know — risks, follow-ups, things deliberately scoped out. -->
