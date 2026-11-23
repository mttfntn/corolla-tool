# Handover: Corolla

## What it is

Corolla is a small desktop app for Windows and macOS that lives in the tray (menu bar on a
Mac). When you drag files with Ctrl held (Cmd on a Mac), a wheel of circles appears at the
cursor; dropping the files on a circle converts, compresses or resizes them. Results go next to
the originals and never overwrite anything.

It's a [Tauri 2](https://tauri.app) app: the logic is Rust (`src-tauri/src`), the wheel and the
Settings window are plain HTML, CSS and JavaScript (`src`). The converting itself is done by
existing programs ("engines") that the installer carries: FFmpeg, ImageMagick, 7-Zip, PDFium,
Assimp, Mayo and FontForge on Windows; on macOS only 7-Zip, PDFium and Assimp are carried, and
the rest is found when the user installs it (README, "macOS: what to install").

Version 0.3.0 is the first one on GitHub. The repository is **private**:
<https://github.com/mttfntn/corolla-tool>. While it's private, the README badges are static
(update the release badge by hand for each release), Actions minutes count against the
account (macOS ten times over, which is why CI tests only Windows on each push), and private
vulnerability reporting, which SECURITY.md points to, isn't available. To make it public, see
"Going public" below.

## What you need

- **Node.js 22** and npm, for the Tauri command-line tool (`npm ci`).
- **Rust**, a recent stable release (CI uses the latest stable).
- **Windows:** Visual Studio 2022 or later C++ build tools (for Rust and for building Assimp),
  and CMake (comes with Visual Studio). PowerShell for `scripts/fetch-engines.ps1`.
- **macOS 13 or newer on Apple silicon:** the Xcode command line tools and CMake
  (`brew install cmake`). For a full local test also `brew install ffmpeg imagemagick
  fontforge`.
- The engines, fetched once by `scripts/fetch-engines.ps1` (Windows) or
  `scripts/fetch-engines.sh` (macOS) into `src-tauri/binaries/<os>`. They are not in git
  (`.gitignore`), and downloads are cached in `build-cache`.

## How the code is organized

| Path | What it does |
|---|---|
| `src-tauri/src/main.rs` | Starts the app: finds the engines, loads settings, builds the tray and the queue, wires the wheel window's drag-and-drop events, and runs the gesture thread. Sends the "Converted 3 files" notification. |
| `src-tauri/src/gesture/mod.rs` | The Ctrl/Cmd+drag state machine (`Detector`), shared by both systems and unit-tested. |
| `src-tauri/src/gesture/windows.rs`, `macos.rs` | One `sample()` each: mouse button, Ctrl (Windows) or Cmd (macOS), cursor position. |
| `src-tauri/src/wheel.rs` | Owns the overlay window: where it is, which circles it shows, which one the cursor is on, and the progress display. Ring layout and hit testing. |
| `src-tauri/src/tools.rs` | Which circles a drag gets: conversions from `formats.json`, plus `CMPRSS`, `RSZ`, `ICO`, `PDF`, and the user's presets; their order and on/off from Settings. |
| `src-tauri/formats.json`, `src-tauri/src/formats.rs` | The conversion matrix: format families, aliases (`jpeg` → `jpg`) and targets. |
| `src-tauri/src/queue.rs` | Worker threads (CPU cores − 1, 1 to 8), progress, cancel, and the report at the end of a batch. |
| `src-tauri/src/output.rs` | Names the result beside the source and reserves it, never overwriting (`name (1).ext`). |
| `src-tauri/src/settings.rs` | `settings.json` in the app's config folder, and the commands the Settings window calls. |
| `src-tauri/src/tray.rs`, `logfile.rs` | The tray menu; the log (`corolla.log` in the app's log folder). |
| `src-tauri/src/engines/mod.rs` | Finds the engines (`Tools::locate`), asks FFmpeg for its encoders, decides what this machine can convert (`can_convert`, `can_process`, `can_combine`), and runs a job: reserve the name, convert in a private temp folder, check the result, move it beside the source. |
| `src-tauri/src/engines/*.rs` | One file per family: `images`, `media` (audio and video), `pdf`, `pdf_compress`, `pdf_writer`, `docx`, `textpages`, `subtitles`, `archives`, `mesh3d`, `step`, `fonts`; `signature` checks a result really is the format its name says. |
| `src-tauri/src/engines/tests.rs`, `abuse_tests.rs` | Engine tests (ignored by default, they need the engines): every pair in the matrix, hostile input, and the behaviour probe. |
| `src/index.html`, `wheel.js`, `wheel-draw.js`, `wheel.css` | The wheel overlay. |
| `src/settings.html`, `settings.js`, `settings.css` | The Settings window. |
| `src-tauri/tauri.conf.json`, `tauri.windows.conf.json`, `tauri.macos.conf.json` | App and installer settings; the per-system files add the engines folder, NSIS with the offline WebView2 on Windows, and DMG, ad hoc signing and macOS 13 minimum on macOS. |
| `scripts/fetch-engines.ps1`, `fetch-engines.sh` | Download (and for Assimp, build) the engines and their licenses. |
| `scripts/release-mac.sh` | Builds, tests and checks the DMG on a Mac and adds it, with the extras installer, to the release. |
| `macos/Install Corolla Extras.command` | The double-click installer for the free tools on a Mac (Homebrew, FFmpeg, ImageMagick, FontForge, FreeCAD, rar; LibreOffice and Blender on request). `DRY_RUN=1` prints instead of installing. Shipped zipped, which keeps it executable. |
| `scripts/make-icon.py` | Draws the app icon (`python3 scripts/make-icon.py out.png`). Not shipped. |
| `tools/check_package.py` | Opens a built DMG or NSIS installer and checks engines, licenses, signature; prints a contents digest. |
| `tests/drive-gesture.ps1`, `drive-explorer.ps1` | Windows only: real drags with synthetic input, to test the gesture. They take over the mouse. |
| `tests/fixtures` | Test files: three fonts (OFL), two STEP files. |
| `.github/workflows/tests.yml` | CI: unit tests on Windows for each push; for a tag, the Windows engines, installer and the release. |

## How the main feature works

1. **Gesture.** A thread in `main.rs` calls `gesture::sample()` every 25 ms (8 ms while a
   button is down). `Detector::step` turns the samples into signals: `Arm` once the button
   has moved 6 px with Ctrl/Cmd held for 120 ms, `Follow` as the cursor moves, `Expired` if
   no files arrive, `Release` when the button comes up.
2. **Catching the files.** On `Arm`, `wheel::Controller` puts the wheel window, still
   invisible, under the cursor. The operating system's own drag-and-drop then reports the
   dragged files to that window (`DragDrop::Enter` in `main.rs`). This is why no special
   permission is needed: Corolla never reads another app's drag directly.
3. **Showing the wheel.** `on_drag_enter` works out the circles (`items_for` in `wheel.rs`, with `tools.rs`):
   targets common to all dragged files that this machine can do (`engines::can_convert`), plus
   tools. `ring()` lays them out, and the front end draws them (`wheel:open` event).
4. **Dropping.** `on_drop` finds the circle under the cursor (`circle_at`) and gives the files
   and the action to the queue. Dropping elsewhere just closes the wheel.
5. **Converting.** Each worker takes a job and calls `engines::convert`: reserve the output
   name beside the source, convert into a private temp folder with the right engine
   (`dispatch`), check the result (`signature::check_file`, for 3D also `has_geometry`), and
   move it into place. The wheel shows progress meanwhile; at the end, a notification and the
   log say what happened.

What the wheel offers depends on the engines found: bundled ones first, then installed ones
(on PATH, plus Homebrew's folders on macOS, plus known install folders for Blender, LibreOffice,
FreeCAD, FontForge, WinRAR). FFmpeg is asked for its encoders at startup, so a target its build
can't write isn't offered. On macOS without ImageMagick, `sips` does JPG, PNG, TIFF, BMP and
HEIC, and FFmpeg the other pictures (`images.rs`, `native_raster_to`).

## How to make a change

1. Read the files involved and the tests next to them. Most modules have unit tests at the
   bottom.
2. For a new conversion, add it to `src-tauri/formats.json`; if it needs a new engine, add it
   to `Tools` (`engines/mod.rs`), to `can_convert`, and to `dispatch`, and to both fetch
   scripts if it's bundled (with its license, see below).
3. Run the unit tests and clippy; with the engines fetched, the engine tests too (next
   section). For a bug fix, first write a test that fails without the fix.
4. If behaviour changes on purpose, compare with the previous version (next section) and
   explain every difference in the commit message.
5. Try the gesture by hand on the system you changed: `npm run dev`.

## How to run the tests

From the repository root:

```bash
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

Expected: `122 passed; 0 failed; 22 ignored` (or more), and no clippy warnings.

The engine tests need the engines (`scripts/fetch-engines.*` first):

```bash
cargo test --manifest-path src-tauri/Cargo.toml -- --ignored --nocapture
```

On Windows with everything fetched, all of them should pass. On a Mac, ten need ImageMagick,
two FontForge and one FreeCAD (`brew install imagemagick fontforge`, FreeCAD from its site or
`brew install --cask freecad`); without those they fail at their first line saying so.

The matrix test (`every_pair_in_the_matrix_converts`) can leave engines out, to test the setups
Mac users can have: `COROLLA_TEST_WITHOUT=ffmpeg,magick` (only what the app carries) or
`COROLLA_TEST_WITHOUT=magick` (sips and FFmpeg). Accepted names: `ffmpeg`, `magick`, `sips`,
`blender`, `fontforge`.

**Comparing two versions.** `behaviour_probe` writes every pair's outcome and each output's
size and hash to a file:

```bash
COROLLA_PROBE_OUT=/tmp/new.txt cargo test --manifest-path src-tauri/Cargo.toml behaviour_probe -- --ignored
```

To compare with an older commit, check it out in a worktree
(`git worktree add /tmp/old <commit>`), append the `behaviour_probe` function from
`src-tauri/src/engines/tests.rs` to the old `tests.rs`, link or fetch the same engines into
`/tmp/old/src-tauri/binaries`, run it there with another output file, and compare. Run the new
version twice first: Opus, MKV, WebM, DAE, FBX and archive outputs carry random IDs or times, so
compare those by size (gzip by outcome only).

`tests/drive-gesture.ps1` and `tests/drive-explorer.ps1` (Windows) test the gesture itself with
real synthetic drags; the macOS gesture is only tested by hand.

## How to make a release

1. Raise the version in `package.json`, `src-tauri/Cargo.toml` and
   `src-tauri/tauri.conf.json`, then `npm install --package-lock-only` (the unit test
   `every_file_states_the_same_version` checks they agree). Commit and push; wait for a green
   CI run.
2. Tag with the notes as the tag's message, and push the tag:

   ```bash
   git tag -a v0.3.1 -m "Corolla 0.3.1

   - What changed, in plain words."
   git push origin v0.3.1
   ```

3. CI tests again on Windows, builds `Corolla-<version>-windows-x64-setup.exe`, checks the
   tag against `package.json`, and publishes the release with the tag's message as notes
   (about 25 minutes the first time, less once the engines are cached). Re-running it replaces
   the Windows file instead of failing.
4. On a Mac, with the tag checked out: `bash scripts/release-mac.sh`. It fetches the engines,
   runs clippy, the unit tests and the engine tests that need nothing installed, builds and
   checks the DMG, and adds `Corolla-<version>-macos-arm64.dmg` and
   `Corolla-<version>-macos-extras.zip` to the release (creating it if CI hasn't yet).
5. Update the release badge in the README by hand (the repository is private).
6. Download the files from the release and check them:
   `python3 tools/check_package.py <file>` (the Windows check needs `src-tauri/binaries/windows`
   fetched, as it uses that 7-Zip).

The macOS CI job can still be started by hand (Actions > tests > Run workflow, tick
"macos"): it tests with Homebrew's tools in three setups and builds the DMG, but uses ten
times the minutes.

The Windows engines are the latest releases at build time (Mayo and FontForge are pinned by
version and checksum). To pin another one, give it a fixed URL and checksum in
`fetch-engines.ps1`, as Mayo has.

## Going public

Check the history again for anything private (`git log -p`), the commit emails, and that CI
is green. Then switch the README's static badges back to live ones (shields.io's
`github/v/release`, `github/license`, `github/languages/top`), make the repository public,
switch on private vulnerability reporting (Settings > Code security), and check every badge.
Public repositories get Actions minutes free, so the macOS job could then run on every tag
again (`if:` in `tests.yml`).

## Updating the engines

- **Windows:** delete the engine from `src-tauri/binaries/windows` (and its folder in
  `build-cache`) and run `fetch-engines.ps1` again: it downloads the latest release. For Mayo
  and FontForge, change the version and the SHA-256 in the script.
- **macOS:** the same with `src-tauri/binaries/macos` and `fetch-engines.sh` (cache folders
  end in `-mac`).
- Every bundled engine needs its license copied into `binaries/<os>/licenses` by the script,
  and a line in `THIRD-PARTY-NOTICES.md`. `tools/check_package.py` fails if a listed license is
  missing.
- After updating, run the engine tests and the behaviour probe against the previous engines.

## Third-party code

The engines and their licenses are listed in [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

Rust libraries used directly (`src-tauri/Cargo.toml`): `tauri` and its plugins
`single-instance`, `notification`, `autostart`, `opener` (MIT or Apache-2.0); `serde`,
`serde_json` (MIT or Apache-2.0); `zip` (MIT); `imagesize` (MIT); `image` (MIT or Apache-2.0);
`pdfium-render` (MIT or Apache-2.0); `resvg`, `usvg`, `tiny-skia`, `svg2pdf` (MIT or
Apache-2.0); `lopdf` (MIT); `ttf2woff2` (MIT or Apache-2.0); `wuff` (MIT or Apache-2.0);
`flate2` (MIT or Apache-2.0); `ttf-parser` (MIT or Apache-2.0); `windows-sys` (MIT or
Apache-2.0). Including everything they use, 366 crates on macOS and 367 on Windows; their
licenses are MIT, Apache-2.0, BSD, Zlib, ISC, Unicode-3.0, MPL-2.0, Unlicense, CC0 or BSL, all
compatible with Corolla's GPL-3.0-or-later. To list them:
`cargo metadata --manifest-path src-tauri/Cargo.toml --format-version 1`.

Test fixtures: `font-static.ttf` is Lato 2.015, `font-cff.otf` is Source Sans 3 (3.052), and
`font-variable.ttf` is Jost 3.710, each under the SIL Open Font License 1.1 (license files
beside them). `sample.step` is `tests/inputs/cube.step` from Mayo (BSD-2-Clause), byte for
byte. `curved.stp` (a cylinder and two cones) was made with Open CASCADE for this project; its
exact origin isn't recorded.

## Things to know

- **Each installer has to be built on its own system.** CI does both.
- **Unsigned.** Without paid certificates, Windows SmartScreen and macOS Gatekeeper warn on
  first start; the README says how to get past them. The Mac app is signed ad hoc as a whole
  (`signingIdentity: "-"`), which is what makes "Open Anyway" possible: with only the linker's
  signature, macOS calls a downloaded app damaged.
- **macOS finds Homebrew tools itself.** An app opened from the Finder doesn't get the shell's
  PATH, so `engines/mod.rs` also searches `/opt/homebrew/bin` and `/usr/local/bin`.
- **FFmpeg builds differ.** Homebrew's has no `libvorbis`, `libwebp` or `libaom-av1`; the
  gyan.dev Windows build has them all. `encoders_needed` in `engines/mod.rs` lists which
  encoders each target needs and must follow `media.rs` and `images.rs`.
- **Minimum macOS is 13**, because of the bundled PDFium; `fetch-engines.sh` builds Assimp for
  13.0 too (CMake would otherwise target only the Mac doing the build).
- **Apple silicon only.** The Mac engines are arm64. An Intel build would need x86_64 or
  universal engines and a `--target` for the app.
- **Lock order in `wheel.rs`:** never call a window method while holding `phase` (the comment
  at the top explains the deadlock).
- **The wheel window starts visible but off-screen** and is then hidden: on Windows the drop
  handler only attaches properly to a webview created while shown.
- **Engines' output is checked**, because exit codes lie: ImageMagick without HEIC writes a PNG
  named `.heic` and succeeds; Assimp exports an empty model from a broken OBJ and succeeds.
- **`build-cache` is shared** by both fetch scripts; the macOS one uses `-mac` folder names
  because the Windows one empties its own folders before unpacking.
- **Windows console:** `tools/check_package.py` prints ASCII only.
- **FontForge's uninstaller can hang** on GitHub's Windows machines (it did, for 69 minutes);
  `fetch-engines.ps1` stops it after 2 minutes.
- **CI caches the Windows engines** by the contents of `fetch-engines.ps1`,
  `THIRD-PARTY-NOTICES.md` and `LICENSE`; change the script (for instance a pinned version) to
  fetch new ones.

## Known limits

The user-facing ones are in the README ("Known limits", "macOS: what to install"). Also:

- The macOS Cmd+drag gesture, the Settings window and the tray on macOS have not been tried by
  hand yet: the code builds and starts, finds its engines, and the conversions are tested, but
  nobody has dragged a file in the Finder with it yet.
- FreeCAD and FontForge on macOS are found at the paths in `step::find_freecad` and
  `fonts::find_fontforge`; the FontForge one is checked by CI (Homebrew), the FreeCAD one isn't.
- On a Mac without ImageMagick, `CMPRSS` on PNG and BMP rarely makes them smaller (only
  ImageMagick reduces colours), so it usually writes nothing.
- An archive holding a symlink to a file outside it fails with 7-Zip's message rather than a
  plain explanation.
- The gesture thread keeps checking the mouse (every 25 ms) even when Corolla is switched off in
  the tray. Measured at about 0.2% of one CPU core on the debug build; not changed.
