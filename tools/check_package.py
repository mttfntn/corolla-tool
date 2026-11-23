"""Opens a built installer and checks that it carries the engines and licenses.

Usage:
    python tools/check_package.py <Corolla_x.y.z_aarch64.dmg | Corolla_x.y.z_x64-setup.exe>

macOS: mounts the DMG read-only, checks the app's engines, licenses, minimum macOS and
signature. Windows: unpacks the NSIS installer with the bundled 7-Zip and checks the engines
and licenses. Both print a digest of the app's contents (file names and data, not the
installer's own bytes, which change with compression and signing), so two builds of the same
commit can be compared. Prints ASCII only, so a Windows console can't stop it.
"""

import hashlib
import pathlib
import plistlib
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent

COMMON_LICENSES = ["LICENSE", "THIRD-PARTY-NOTICES.md", "7-Zip.txt", "PDFium.txt", "Assimp.txt"]
MAC_ENGINES = ["7zz", "assimp", "libpdfium.dylib"]
WINDOWS_ENGINES = [
    "7z.exe", "ffmpeg.exe", "ffprobe.exe", "magick.exe", "pdfium.dll", "assimp.exe",
    "vcomp140.dll", "mayo/mayo-conv.exe", "fontforge/bin/fontforge.exe",
]
WINDOWS_LICENSES = [
    "FFmpeg.txt", "ImageMagick.txt", "FontForge.txt", "Mayo.txt", "OpenCascade-LGPL-2.1.txt",
    "OpenCascade-exception.txt", "Qt-LGPL-3.0.txt", "FreeImage.txt",
]


def digest(folder: pathlib.Path) -> tuple[str, int]:
    """SHA-256 over every file's relative path and contents, in sorted order."""
    total = hashlib.sha256()
    count = 0
    for path in sorted(p for p in folder.rglob("*") if p.is_file()):
        total.update(path.relative_to(folder).as_posix().encode())
        total.update(b"\0")
        total.update(hashlib.sha256(path.read_bytes()).digest())
        count += 1
    return total.hexdigest(), count


def require(folder: pathlib.Path, names: list[str], what: str) -> list[str]:
    missing = [name for name in names if not (folder / name).is_file()]
    print(f"{what}: {len(names) - len(missing)} of {len(names)} present")
    return [f"{what}: {name}" for name in missing]


def check_mac(dmg: pathlib.Path) -> list[str]:
    problems = []
    with tempfile.TemporaryDirectory() as mount:
        subprocess.run(["hdiutil", "attach", "-nobrowse", "-readonly", "-mountpoint", mount, str(dmg)],
                       check=True, stdout=subprocess.DEVNULL)
        try:
            app = pathlib.Path(mount) / "Corolla.app"
            binaries = app / "Contents/Resources/binaries"
            problems += require(binaries, MAC_ENGINES, "engines")
            problems += require(binaries / "licenses", COMMON_LICENSES, "licenses")
            info = plistlib.loads((app / "Contents/Info.plist").read_bytes())
            minimum = info.get("LSMinimumSystemVersion")
            print(f"minimum macOS: {minimum}")
            if minimum != "13.0":
                problems.append(f"LSMinimumSystemVersion is {minimum}, expected 13.0")
            # The whole bundle must be signed (ad hoc), or a downloaded copy is called damaged.
            verify = subprocess.run(["codesign", "--verify", "--deep", "--strict", str(app)],
                                    capture_output=True, text=True)
            print(f"signature: {'sealed' if verify.returncode == 0 else 'NOT sealed'}")
            if verify.returncode != 0:
                problems.append(f"codesign: {verify.stderr.strip()}")
            value, count = digest(app)
            print(f"contents digest: {value} ({count} files)")
        finally:
            subprocess.run(["hdiutil", "detach", mount], check=False, stdout=subprocess.DEVNULL)
    return problems


def check_windows(setup: pathlib.Path) -> list[str]:
    seven = ROOT / "src-tauri/binaries/windows/7z.exe"
    with tempfile.TemporaryDirectory() as out:
        subprocess.run([str(seven), "x", "-y", f"-o{out}", str(setup)], check=True, stdout=subprocess.DEVNULL)
        binaries = next((p for p in pathlib.Path(out).rglob("binaries") if p.is_dir()), None)
        if binaries is None:
            return ["no binaries folder in the installer"]
        problems = require(binaries, WINDOWS_ENGINES, "engines")
        problems += require(binaries / "licenses", COMMON_LICENSES + WINDOWS_LICENSES, "licenses")
        value, count = digest(binaries)
        print(f"engines digest: {value} ({count} files)")
    return problems


def main() -> int:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(errors="replace")
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    package = pathlib.Path(sys.argv[1])
    size = package.stat().st_size / 1024 ** 2
    print(f"{package.name}: {size:.1f} MB")
    problems = check_mac(package) if package.suffix == ".dmg" else check_windows(package)
    for problem in problems:
        print(f"MISSING {problem}")
    print("package OK" if not problems else f"{len(problems)} problem(s)")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
