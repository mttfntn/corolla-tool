## What this changes

<!-- What does the pull request do, and why? Link the issue it fixes, e.g. "Fixes #12". -->

## How it was tested

- [ ] `cargo test --manifest-path src-tauri/Cargo.toml` ends with `0 failed`
- [ ] `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` is clean
- [ ] Engine tests (`-- --ignored`) run with the engines fetched, on: <!-- Windows / macOS -->
- [ ] For a bug fix: a new test that fails without the fix
- [ ] Behaviour compared with the previous version (`behaviour_probe`), every difference explained
- [ ] Tried by hand with `npm run dev` (if it changes the gesture, the wheel or Settings), on: <!-- Windows / macOS -->

## Checklist

- [ ] Comments match the code
- [ ] README updated if users will notice the change
- [ ] A new bundled engine comes with its license (fetch script, THIRD-PARTY-NOTICES.md, tools/check_package.py)
- [ ] Version bumped in `package.json`, `package-lock.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json` (releases only)
