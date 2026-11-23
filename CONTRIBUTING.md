# Contributing to Corolla

Thanks for helping! Bug reports, ideas and pull requests are all welcome.

## Reporting a bug or suggesting a feature

Open an [issue](https://github.com/mttfntn/corolla-tool/issues/new/choose) and pick a form:

- **Bug report:** say which Corolla version (the first line of the log: tray icon > **Open log
  folder** > `corolla.log`), which system, which files you dropped on which circle, and what
  happened. Paste the end of the log.
- **Feature request:** describe what you're trying to do, not only the solution you have in mind.

First, check whether someone has already opened an issue for the same thing, and read the
README's [known limits](https://github.com/mttfntn/corolla-tool#known-limits) and, on a Mac,
[what to install](https://github.com/mttfntn/corolla-tool#macos-what-to-install).
Security problems go through [SECURITY.md](SECURITY.md), not public issues.

## Changing the code

[HANDOVER.md](HANDOVER.md) explains how the project is organized and the pitfalls already found.
Read it before a larger change.

1. **Fork** the repository and create a branch for your change.
2. You need Node.js 22, Rust (stable) and the build tools for your system (README, "Build from
   source"). Run `npm ci`, then the fetch script for your system to get the engines.
3. Run the tests from the repository root:

   ```
   cargo test --manifest-path src-tauri/Cargo.toml
   cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
   cargo test --manifest-path src-tauri/Cargo.toml -- --ignored
   ```

   Expected: the unit tests end with `0 failed` (122 or more) and clippy prints no warnings.
   The engine tests (`--ignored`) all pass on Windows; on a Mac some need ImageMagick,
   FontForge or FreeCAD (HANDOVER.md, "How to run the tests").
4. **If you fix a bug,** add a test that fails without your fix.
5. **If you change behaviour on purpose,** compare with the previous version using the
   behaviour probe (HANDOVER.md) and explain every difference in the pull request.
6. **Try it by hand** with `npm run dev` if you change the gesture, the wheel or Settings.
7. **Open a pull request.** The template has a short checklist. CI runs the tests on Windows and
   macOS, the engine tests, and builds both installers.

### Style

- The front end stays plain HTML, CSS and JavaScript, with no build step and no libraries.
- Never overwrite the user's files, and check an engine's output rather than trusting its exit
  code.
- A bundled engine comes with its license (fetch script, `THIRD-PARTY-NOTICES.md`,
  `tools/check_package.py`).
- Comments must say what the code does; check them when you change the code.
- Keep commits small, with messages that explain what changed and how you checked it.

## License

Corolla is GPL-3.0-or-later. By contributing, you agree that your contribution is released under
the same license.
