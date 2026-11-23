# Corolla

[![Platforms](https://img.shields.io/badge/platform-Windows%2010%2B%20%7C%20macOS%2013%2B-555)](#download)
[![Release](https://img.shields.io/github/v/release/mttfntn/corolla-tool)](https://github.com/mttfntn/corolla-tool/releases/latest)
[![Tests](https://github.com/mttfntn/corolla-tool/actions/workflows/tests.yml/badge.svg)](https://github.com/mttfntn/corolla-tool/actions/workflows/tests.yml)
[![License](https://img.shields.io/github/license/mttfntn/corolla-tool)](LICENSE)
[![Language](https://img.shields.io/github/languages/top/mttfntn/corolla-tool)](src-tauri/src/main.rs)
[![Engines](https://img.shields.io/badge/engines-FFmpeg%20%C2%B7%20ImageMagick%20%C2%B7%207--Zip%20%C2%B7%20PDFium%20%C2%B7%20Assimp-blue)](THIRD-PARTY-NOTICES.md)

Drag files with **Ctrl** held (**Cmd** on a Mac) and a wheel of circles appears at the
cursor. Drop on a circle to convert, compress or resize them. It handles batches, and the
circles depend on the file type.

- **Windows:** hold Ctrl and drag, or start dragging and then press Ctrl.
- **macOS:** hold Cmd and drag, or start dragging and then press Cmd.

Drop anywhere except a circle to cancel. A grey circle is the format the files already have.
While converting, the centre shows the percentage and how many files are done. Results go next
to the source file. Originals are never changed and nothing is overwritten: name clashes become
`name (1).ext`.

The tray icon (menu bar on a Mac) has Settings, on/off, start at login, quality, cancel, open
last output, and the log folder.

For maintainers: [HANDOVER.md](HANDOVER.md) (people) and [AGENTS.md](AGENTS.md) (AI assistants).

## Download

From the [latest release](https://github.com/mttfntn/corolla-tool/releases/latest):

- **Windows 10 or 11 (64-bit):** `Corolla-<version>-windows-x64-setup.exe`. It contains every
  engine and the WebView2 runtime, so it works offline with nothing else installed.
- **macOS 13 or newer on Apple silicon:** `Corolla-<version>-macos-arm64.dmg`. Open it and drag
  Corolla to Applications. The Mac version carries fewer engines; see
  [macOS: what to install](#macos-what-to-install).

Corolla isn't signed with a paid Apple or Microsoft certificate, so the first start needs one
extra step:

- **Windows:** if SmartScreen says "Windows protected your PC", choose **More info**, then
  **Run anyway**.
- **macOS:** open Corolla once; when macOS refuses, go to **System Settings > Privacy &
  Security**, scroll down and choose **Open Anyway** (on macOS 13 and 14, right-clicking the
  app and choosing **Open** also works).

## Tools

- **`CMPRSS`** makes images, PDFs and videos smaller in the same format: `photo_cmprss.jpg`. If
  the result wouldn't be smaller, no file is written. PDFs keep their text and vector graphics,
  only the pictures inside are shrunk, so a PDF with few pictures won't change much. With
  ImageMagick, PNG and BMP reduce to 256 colours below quality 90, because they have no other
  way to shrink much.
- **`ICO`** makes a favicon: one file holding 16, 32 and 48 px for browsers, and 256 px so it
  also works as a Windows icon. A picture that isn't square is centred on a transparent square,
  not stretched. Dropping an ICO uses its largest size. Needs ImageMagick.
- **`RSZ /2`, `RSZ /4`** shrink images: `photo_rsz2.jpg`.
- **`PDF`** with several images makes one PDF with a page per image, in filename order (`img2`
  before `img10`), named after the folder. **`PDF EACH`** makes one PDF per image.

In **Settings** (tray icon, or open Corolla again while it's running), drag circles to swap
their places, switch off the ones you don't use, and add your own compress and resize circles
under Presets.

## Formats

| Family | Formats |
|---|---|
| Images | JPG PNG WEBP HEIC TIFF AVIF BMP EXR HDR ICO, SVG (input only), plus output to PDF; JPG and PNG also to DOCX |
| PDF | to DOCX JPG PNG TXT |
| Audio | MP3 M4A WAV FLAC OGG OPUS AIFF WMA |
| Video | MP4 MOV MKV WEBM AVI WMV GIF, plus audio to MP3 |
| Subtitles and text | SRT VTT TXT; TXT also to PDF JPG PNG |
| Archives | ZIP TAR GZIP RAR |
| 3D | FBX GLB GLTF DAE OBJ STL; STEP and STP (input only) |
| Fonts | TTF OTF WOFF WOFF2 |

The wheel only offers what the engines on this computer can do, down to the encoders the
installed FFmpeg has. To add or change a conversion, edit `src-tauri/formats.json`.

## macOS: what to install

The Mac app carries 7-Zip, PDFium and Assimp, and uses macOS's own `sips` for pictures. Out of
the box that covers archives, PDF, 3D models, WOFF and WOFF2 fonts, subtitles and text, and
JPG, PNG, TIFF, BMP and HEIC pictures (between each other, to PDF, and from SVG). The rest needs
free tools that Corolla finds once they're installed. The easiest way is
[Homebrew](https://brew.sh); after installing, quit Corolla from the menu bar and open it again.

| To get | Install |
|---|---|
| Audio and video; EXR and HDR pictures; reading WebP and AVIF | `brew install ffmpeg` |
| ICO; writing WebP and AVIF; PNG and BMP compression that works | `brew install imagemagick` |
| TTF ↔ OTF fonts | `brew install fontforge` |
| STEP and STP files | `brew install --cask freecad` |
| Making RAR archives | `brew install --cask rar` |
| PDF to DOCX with its layout (not just text) | `brew install --cask libreoffice` |
| Better FBX rigs and animation | `brew install --cask blender` |

```bash
brew install ffmpeg imagemagick fontforge
```

Homebrew's FFmpeg has no Vorbis encoder, so OGG stays hidden as a target even with it
installed (OGG files can still be converted to other formats). Corolla looks in Homebrew's
folders (`/opt/homebrew/bin`, `/usr/local/bin`), and in `/Applications` for FreeCAD, Blender
and LibreOffice.

## Known limits

- EXR and HDR hold linear light, and values above 1.0 survive between EXR, HDR and TIFF, which
  comes out as 32-bit float. Going to JPG, PNG, WEBP, AVIF, BMP or PDF converts to sRGB and
  clips anything brighter than white, without tone mapping. Only the first image in an EXR is
  used, not extra layers or passes such as depth. `CMPRSS` on EXR is lossless ZIP; HDR has no
  `CMPRSS` circle.
- Writing HEIC only works on macOS. On Windows, HEIC files can be converted from, but the HEIC
  circle is hidden, because the bundled ImageMagick has no HEIC encoder.
- Creating RAR needs WinRAR or `rar` installed.
- STL has no materials or animation, and OBJ has no rigs or animation. A model with no faces at
  all (for instance a broken file) fails instead of giving an empty result.
- If Blender is installed, FBX conversions use it for better rigs and animation; otherwise
  Assimp does them.
- PDF to DOCX is text only unless LibreOffice is installed.
- TXT to PDF uses a built-in font: characters outside Western European sets become `?`, and the
  log says so.
- STEP becomes a mesh of triangles. There's no mesh to STEP, as that wouldn't give a real CAD
  solid. On macOS, STEP needs FreeCAD, because Mayo has no Mac release: FreeCAD makes an STL,
  and Assimp turns that into the other 3D formats.
- Fonts: WOFF and WOFF2 are compressed wrappers, so to and from them nothing is lost. TTF to OTF
  and back redraws the letters' curves with FontForge: names, copyright, weight and every
  character are kept, but TrueType hinting is lost going to OTF, and going to TTF adds the two
  technical glyphs TrueType requires. Variable fonts can't go between TTF and OTF, because only
  one of their styles would survive; WOFF and WOFF2 keep them whole.
- An archive holding a link to a file outside itself can't be converted: 7-Zip won't follow
  it, so its contents can't leak into the result.
- SLDPRT isn't supported, as no free library reads it reliably.
- macOS: only Apple silicon Macs, as the engines are built for arm64. The Mac version is new,
  so please [report](https://github.com/mttfntn/corolla-tool/issues/new/choose) anything that
  behaves differently from Windows.

## Build from source

You need Node.js 22, Rust (stable) and, on Windows, the Visual Studio 2022 C++ build tools.
On macOS, the Xcode command line tools and CMake (`brew install cmake`).

```bash
npm ci
```

Fetch the engines once. They aren't kept in the repository, and the scripts skip what's
already there. On Windows:

```bash
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/fetch-engines.ps1
```

On macOS:

```bash
bash scripts/fetch-engines.sh
```

Run Corolla from the source, or build the installer (NSIS on Windows, DMG on macOS, in
`src-tauri/target/release/bundle`):

```bash
npm run dev
```

```bash
npm run build
```

Each installer has to be built on its own system: Windows on Windows, macOS on a Mac. CI does
both for every release.

## Tests

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Every pair in `formats.json` through the real engines, plus hostile input (odd file names,
broken files, archives that try to escape their folder). Run the fetch script first:

```bash
cargo test --manifest-path src-tauri/Cargo.toml -- --ignored --nocapture
```

`tests/drive-gesture.ps1` and `tests/drive-explorer.ps1` perform real Windows drags with
synthetic input to test the gesture itself. They take over the mouse for a few seconds.
More in [HANDOVER.md](HANDOVER.md#how-to-run-the-tests).

## License

[GPL-3.0-or-later](LICENSE). © Matteo Fontana. The engines Corolla carries keep their own
licenses: see [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).
