# Third-party software in Corolla

Corolla itself is licensed under the GNU General Public License, version 3 or later
([LICENSE](LICENSE)). The installer and the app also carry these programs, which do the
actual converting. Each keeps its own license. The full license texts are in the `licenses`
folder next to the engines: inside the app, `binaries/licenses` on Windows and
`Contents/Resources/binaries/licenses` on macOS. The build scripts
(`scripts/fetch-engines.ps1` and `scripts/fetch-engines.sh`) download each one from the source
listed here and put its license there.

| Program | Used for | License | Source |
|---|---|---|---|
| FFmpeg (Windows: the gyan.dev "essentials" build) | audio, video, some images | GPL-3.0-or-later (built with GPL libraries such as x264) | <https://ffmpeg.org>, build and its exact sources: <https://www.gyan.dev/ffmpeg/builds/> |
| ImageMagick (Windows, portable Q16 HDRI) | images | ImageMagick License | <https://imagemagick.org>, <https://github.com/ImageMagick/ImageMagick> |
| 7-Zip | archives | GNU LGPL-2.1-or-later, parts BSD-3-Clause, with the unRAR restriction | <https://www.7-zip.org>, <https://github.com/ip7z/7zip> |
| PDFium (pdfium-binaries build) | reading PDFs | BSD-3-Clause and Apache-2.0; its own third-party licenses are included | <https://pdfium.googlesource.com/pdfium/>, build: <https://github.com/bblanchon/pdfium-binaries> |
| Assimp | 3D models | BSD-3-Clause | <https://github.com/assimp/assimp> |
| Mayo 0.10.0 (Windows) | STEP files | BSD-2-Clause | <https://github.com/fougue/mayo> |
| Open CASCADE Technology 7.9.3 (inside Mayo) | STEP files | GNU LGPL-2.1 with the OCCT exception | <https://github.com/Open-Cascade-SAS/OCCT> |
| Qt 5.15 Core (inside Mayo) | used by Mayo | GNU LGPL-3.0 | <https://download.qt.io/archive/qt/5.15/> |
| FreeImage (inside Mayo) | used by Mayo | FreeImage Public License 1.0 (or GPL) | <https://freeimage.sourceforge.io> |
| FontForge (Windows) | TTF and OTF fonts | GPL-3.0-or-later, parts BSD-3-Clause; carries its own Python under the PSF License | <https://fontforge.org>, <https://github.com/fontforge/fontforge> |
| Microsoft OpenMP runtime `vcomp140.dll` (Windows) | needed by ImageMagick | Microsoft Visual C++ Redistributable terms | <https://learn.microsoft.com/cpp/windows/latest-supported-vc-redist> |
| Microsoft Edge WebView2 Runtime (Windows installer) | draws Corolla's windows | Microsoft Software License Terms | <https://developer.microsoft.com/microsoft-edge/webview2/> |

On macOS, Corolla also uses `sips`, which is part of macOS, and, when they are installed,
FFmpeg, ImageMagick, FontForge, FreeCAD, Blender, LibreOffice and `rar`. Those are not
included in the app; each keeps its own license.

The Rust libraries Corolla is built from are listed with their licenses in
[HANDOVER.md](HANDOVER.md#third-party-code); all are MIT, Apache-2.0, BSD, Zlib, ISC,
Unicode, MPL-2.0 or similar, and compatible with the GPL-3.0.

The test files in `tests/fixtures` are not part of the app: the fonts are Lato, Source Sans 3
and Jost under the SIL Open Font License 1.1 (license files beside them), and `sample.step`
is `tests/inputs/cube.step` from Mayo (BSD-2-Clause).
