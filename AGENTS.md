# AGENTS.md: Corolla

Tauri 2 tray app for Windows and macOS: Ctrl+drag (Cmd+drag on macOS) files, a wheel of
circles appears, dropping on one converts/compresses/resizes with bundled or installed engines.
Rust in `src-tauri/src`, plain HTML/CSS/JS in `src` (no build step, no libraries).
People-oriented detail: [HANDOVER.md](HANDOVER.md).

## Commands (repo root)

```bash
npm ci                                                     # Tauri CLI
bash scripts/fetch-engines.sh                              # macOS engines -> src-tauri/binaries/macos
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/fetch-engines.ps1   # Windows engines
cargo test --manifest-path src-tauri/Cargo.toml            # unit tests (no engines needed)
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml -- --ignored --nocapture        # engine tests
COROLLA_TEST_WITHOUT=ffmpeg,magick cargo test --manifest-path src-tauri/Cargo.toml every_pair -- --ignored --nocapture
COROLLA_PROBE_OUT=/tmp/probe.txt cargo test --manifest-path src-tauri/Cargo.toml behaviour_probe -- --ignored
npm run dev                                                # run from source
npm run build                                              # NSIS (Windows) / DMG (macOS)
python3 tools/check_package.py <dmg or setup.exe>          # engines, licenses, signature, digest
```

Unit tests need `src-tauri/binaries/<os>` to exist (empty is fine): the bundle config lists it.

## Architecture

- `main.rs`: setup; gesture thread polls `gesture::sample()` (25 ms, 8 ms while pressed) into
  `gesture::Detector` → `Signal`s → `wheel::Controller::on_signal`.
- `wheel.rs`: the one overlay window ("wheel"). `Arm` puts it invisible under the cursor; the
  OS drag-and-drop then reports paths (`DragDrop::Enter`) → `items_for` → `ring()` →
  `wheel:open` event → `on_drop` → `queue.enqueue`. Lock rule: no window calls while holding
  `phase`.
- `tools.rs`: circles per drag (conversions from `formats.json`, `CMPRSS`, `RSZ`, `ICO`, `PDF`,
  presets), ordered/hidden per Settings.
- `queue.rs`: workers (cores − 1, 1..8) → `engines::convert`.
- `engines/mod.rs`: `Tools::locate` (bundled dir, then PATH, then on macOS
  `/opt/homebrew/bin` and `/usr/local/bin`, then known install paths); `ffmpeg_encoders`
  (asked once at startup); `can_convert` / `can_process` / `can_combine` decide what is
  offered; `convert` reserves the output name (`output.rs`), converts in a `WorkDir` temp
  folder via `dispatch`, checks with `signature::check_file`, then moves it beside the source.
- Per family: `images.rs` (ImageMagick, or on macOS `sips` + FFmpeg: `native_raster_to`,
  `native_process`), `media.rs` (FFmpeg), `pdf*.rs` (PDFium, own writer), `archives.rs`
  (7-Zip, `rar`), `mesh3d.rs` (Assimp or Blender; `has_geometry` rejects empty models),
  `step.rs` (Mayo on Windows, FreeCAD on macOS, then Assimp), `fonts.rs` (own WOFF/WOFF2,
  FontForge for TTF↔OTF), `subtitles.rs`, `textpages.rs`, `docx.rs`.
- Front end: `src/wheel*.js` draws the wheel from events; `src/settings.js` calls the Tauri
  commands in `settings.rs` (`get_settings`, `save_settings`, `catalog`, `ring_for`, …).

## Rules and pitfalls

- An engine's exit code isn't proof: check the output (`signature.rs`; for 3D `has_geometry`).
- `encoders_needed` (engines/mod.rs) must list every non-built-in FFmpeg encoder that
  `media.rs` and `images.rs` use for a target; otherwise the wheel offers a target that fails.
- `picture_ok` (engines/mod.rs) must follow `images.rs`'s engine choice: ImageMagick for all;
  without it, sips for `SIPS_FORMATS` pairs, FFmpeg otherwise; PDF pages are JPEGs; SVG is drawn
  to PNG first.
- macOS GUI apps don't get the shell's PATH; keep Homebrew dirs in `EXTRA_DIRS`.
- Every bundled engine: license copied into `binaries/<os>/licenses` by its fetch script, a row
  in `THIRD-PARTY-NOTICES.md`, a name in `tools/check_package.py`.
- `build-cache` is shared; the macOS script uses `*-mac` folders (the Windows one empties
  `7zip`, `pdfium`, `assimp` before unpacking).
- macOS minimum 13.0 (PDFium); Assimp is built with `CMAKE_OSX_DEPLOYMENT_TARGET=13.0`. Engines
  are arm64 only.
- The Mac app must be signed as a whole (`signingIdentity: "-"` in `tauri.macos.conf.json`), or
  a downloaded copy is "damaged".
- Version lives in `package.json`, `package-lock.json`, `src-tauri/Cargo.toml`,
  `src-tauri/tauri.conf.json`; `every_file_states_the_same_version` checks them.
- Engine tests that need ImageMagick/FontForge/FreeCAD fail on their first line without them;
  that's expected on a bare Mac. Use `COROLLA_TEST_WITHOUT` for setup-specific matrix runs.
- Don't add docstrings that the UI shows; there are none of those in this project (Rust doc
  comments aren't shown to users).
- `tests.rs` and Rust sources must stay plain text: write bytes as `\0`, not raw NULs.

## Verification done for 0.3.0 (macOS 26.6, Apple silicon; Homebrew FFmpeg 9.0.2)

- Unit tests: 114 at the baseline commit, 122 now, all passing; clippy clean with `-D warnings`.
- Matrix: with Homebrew FFmpeg 204 passed / 0 failed (baseline: 204 passed / 52 failed: 23
  encoder errors, 29 unmakeable samples); with `COROLLA_TEST_WITHOUT=ffmpeg,magick` 90 passed /
  0 failed (62 before the `picture_ok` fix).
- Hostile input (`abuse_tests.rs`): 5/5; the broken-files test fails on the baseline's
  `mesh3d.rs` (garbage OBJ → empty STL claimed as success).
- Behaviour probe vs baseline (`5af7067`): 288 pairs, 263 identical (34 noisy formats by size,
  gzip by outcome; proven by two runs of the same code), 25 different, all intended
  (23 → hidden for missing encoders, 2 → hidden without a sample).
- FFmpeg encoder query at startup: 0.02 s (median of 5). Idle CPU: about 0.2% (3 runs of 20 s,
  debug build).
- DMG built and checked (`tools/check_package.py`: 13.7 MB, engines and licenses present,
  minimum 13.0, signature sealed); the app copied out of it, started with an empty HOME and the
  Finder's PATH, found its engines and Homebrew's FFmpeg.
- Not verified here: the Windows build and engines (CI runs them), the macOS gesture by hand,
  FreeCAD on macOS.

## Release checklist

1. Bump the version in the four files above (`npm install --package-lock-only` for the lock).
2. Push to `main`; CI green (`unit`, `windows`, `macos`).
3. `git tag -a vX.Y.Z -m "Corolla X.Y.Z" -m "- notes"` and `git push origin vX.Y.Z`.
4. CI publishes `Corolla-X.Y.Z-windows-x64-setup.exe` and `Corolla-X.Y.Z-macos-arm64.dmg`;
   re-running replaces files. Download both and run `tools/check_package.py` on them.

## Git setup

- Only `main`; small commits with messages that say what changed and how it was checked; a fix
  goes in a commit before the tests that check it.
- Author: Matteo Fontana, with the commit email linked to the GitHub account. End commit
  messages with the Co-Authored-By line if an assistant helped.
- Remote: `origin` = https://github.com/mttfntn/corolla-tool (public). The first commit
  (`5af7067`) is the code as it came from the Windows PC: the baseline for comparisons.
- Never commit `src-tauri/binaries`, `build-cache`, `src-tauri/target`, `node_modules`, or
  local assistant/editor setup (`.claude/`, `.mcp.json`, `.vscode/`, `.idea/`).
