# Corolla

Ctrl+drag files anywhere and a wheel of circles appears at the cursor. Drop on a circle to convert, compress or resize. It handles batches, and the circles depend on the file type.

- **Windows:** hold Ctrl and drag, or start dragging and then press Ctrl.
- **macOS:** start dragging, then press Ctrl. Ctrl+click is right-click on a Mac, so holding Ctrl first opens a menu instead of starting a drag.

Drop anywhere except a circle to cancel. A grey circle is the format the files already have. While converting, the centre shows the percentage and how many files are done. Results go next to the source file. Originals are never changed and nothing is overwritten: name clashes become `name (1).ext`.

The tray icon has Settings, on/off, start at login, quality, cancel, open last output, and the log folder.

## Tools

- **`CMPRSS`** makes images, PDFs and videos smaller in the same format: `photo_cmprss.jpg`. If the result wouldn't be smaller, no file is written. PDFs keep their text and vector graphics, only the pictures inside are shrunk, so a PDF with few pictures won't change much. PNG and BMP reduce to 256 colours below quality 90, because they have no other way to shrink much.
- **`ICO`** makes a favicon: one file holding 16, 32 and 48 px for browsers, and 256 px so it also works as a Windows icon. A picture that isn't square is centred on a transparent square, not stretched. Dropping an ICO uses its largest size.
- **`RSZ /2`, `RSZ /4`** shrink images: `photo_rsz2.jpg`.
- **`PDF`** with several images makes one PDF with a page per image, in filename order (`img2` before `img10`), named after the folder. **`PDF EACH`** makes one PDF per image.

In **Settings** (tray icon, or open Corolla again while it's running), drag circles to swap their places, switch off the ones you don't use, and add your own compress and resize circles under Presets.

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

To add or change a conversion, edit `src-tauri/formats.json`. The wheel only offers what this machine's engines can do.

Limits worth knowing:
- EXR and HDR hold linear light, and values above 1.0 survive between EXR, HDR and TIFF, which comes out as 32-bit float. Going to JPG, PNG, WEBP, AVIF, BMP or PDF converts to sRGB and clips anything brighter than white, without tone mapping. Only the first image in an EXR is used, not extra layers or passes such as depth. `CMPRSS` on EXR is lossless ZIP; HDR has no `CMPRSS` circle.
- Writing HEIC only works on macOS. On Windows, HEIC files can be converted from but the HEIC wedge is hidden, because the bundled ImageMagick has no HEIC encoder.
- Creating RAR needs WinRAR or `rar` installed.
- STL has no materials or animation, and OBJ has no rigs or animation.
- If Blender is installed, FBX conversions use it for better rigs and animation; otherwise Assimp does them.
- PDF to DOCX is text only unless LibreOffice is installed.
- TXT to PDF uses a built-in font: characters outside Western European sets become `?`, and the log says so.
- STEP becomes a mesh of triangles. There's no mesh to STEP, as that wouldn't give a real CAD solid. On macOS, STEP needs FreeCAD installed, because Mayo has no Mac release, and then only makes STL directly.
- Fonts: WOFF and WOFF2 are compressed wrappers, so to and from them nothing is lost. TTF to OTF and back redraws the letters' curves with FontForge: names, copyright, weight and every character are kept, but TrueType hinting is lost going to OTF, and going to TTF adds the two technical glyphs TrueType requires. Variable fonts can't go between TTF and OTF, because only one of their styles would survive; WOFF and WOFF2 keep them whole. On macOS, TTF and OTF need FontForge installed.
- SLDPRT isn't supported, as no free library reads it reliably.

## Installing on another PC

Only the installer is needed. It contains the engines and the WebView2 runtime, so it works with no internet and none of the build tools below. Blender, LibreOffice and WinRAR remain optional extras, as described under Limits.

## Build

Needs Node, Rust and, on Windows, the Visual Studio C++ build tools.

```bash
npm install
```

Fetch the engines, once. They aren't kept in the repository.

On Windows:

```bash
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/fetch-engines.ps1
```

On macOS:

```bash
bash scripts/fetch-engines.sh
```

Run it, or build the installer (NSIS on Windows, DMG on macOS):

```bash
npm run dev
```

```bash
npm run build
```

**macOS builds have to be made on a Mac.** Without a paid Apple Developer ID the app is unsigned, so open it the first time with right-click > Open.

## Tests

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Every pair in `formats.json`, through the real engines:

```bash
cargo test --manifest-path src-tauri/Cargo.toml -- --ignored --nocapture
```

`tests/drive-gesture.ps1` and `tests/drive-explorer.ps1` perform real Windows drags with synthetic input to test the gesture itself. They take over the mouse for a few seconds.
