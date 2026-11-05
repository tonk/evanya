#!/usr/bin/env python3
"""Stage desktop and Android downloads with checksums."""

import hashlib
import os
import shutil
from pathlib import Path


def prepare(source: Path, destination: Path, version: str) -> None:
    deb_version = version.replace("-", "~", 1)
    # GitHub rewrites '~' in asset filenames. Normalize before checksumming;
    # the package's internal Debian version keeps '~' for prerelease ordering.
    deb_filename_version = deb_version.replace("~", ".")
    packages = {
        "evanya.AppImage": f"evanya-{version}-linux-x64.AppImage",
        f"evanya_{deb_version}_amd64.deb": f"evanya_{deb_filename_version}_amd64.deb",
        "evanya.rpm": f"evanya-{version}-linux-x64.rpm",
        "evanya.flatpak": f"evanya-{version}-linux-x64.flatpak",
        "evanya-windows-x64.zip": f"evanya-{version}-windows-x64.zip",
        "evanya-windows-x64-setup.exe": f"evanya-{version}-windows-x64-setup.exe",
        "evanya-macos-arm64.zip": f"evanya-{version}-macos-arm64.zip",
        "evanya-macos-arm64.dmg": f"evanya-{version}-macos-arm64.dmg",
        "evanya-macos-x64.zip": f"evanya-{version}-macos-x64.zip",
        "evanya-macos-x64.dmg": f"evanya-{version}-macos-x64.dmg",
        "flectar-mail.apk": f"flectar-mail-{version}-android-arm64.apk",
    }
    # Validate every input before copying anything; unrelated benchmark files
    # are intentionally excluded from the public downloads.
    selected = []
    for name, output_name in packages.items():
        matches = list(source.rglob(name))
        if len(matches) != 1 or not matches[0].is_file() or matches[0].stat().st_size == 0:
            raise ValueError(f"Expected exactly one nonempty {name}, found {len(matches)}")
        selected.append((matches[0], output_name))
    destination.mkdir()  # Refuse to mix new artifacts with a stale dist folder.
    checksums = []
    for path, name in sorted(selected, key=lambda entry: entry[1]):
        output = destination / name
        shutil.copyfile(path, output)
        with output.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        checksums.append(f"{digest}  {name}\n")
    (destination / "SHA256SUMS").write_text("".join(checksums), encoding="utf-8")


def release_notes(version: str) -> str:
    android = (
        "An Android arm64 APK (Android 8.0+) is included. "
        "Starting with alpha 8, previews and stable releases share one signing "
        "identity, so newer versions can update the app without uninstalling it. "
        "Alpha 6 and alpha 7 used different keys and cannot update to this identity. "
        "Android OAuth requires separate registrations tied to the APK signing "
        "identity; desktop keys alone do not enable it.\n"
    )
    return (
        "Desktop builds for Linux x64, Windows x64, and macOS Apple silicon and Intel. "
        "Linux testers can choose AppImage, Debian, Fedora RPM, or Flatpak.\n\n"
        "The x86_64 RPM supports Fedora 43 and 44. The same package is built "
        "on Fedora 43 and tested in clean containers on both releases, including "
        "PDF rendering and X11/Wayland startup. Other RPM distributions are not "
        "yet tested. Install it with `sudo dnf install ./evanya-"
        f"{version}-linux-x64.rpm`; if a graphical installer reports a conflict, "
        "this command shows the dependency details.\n\n"
        "Gmail and Outlook sign-in require a bundled OAuth registration or your own "
        "keys in **Sign-in settings** on the welcome screen. IMAP/SMTP and JMAP "
        "do not require Google or Microsoft app keys. Provider policies still apply.\n\n"
        "Windows downloads are unsigned. macOS downloads are ad-hoc signed, "
        "not Developer ID signed or notarized; OS security prompts are expected. "
        "macOS requires version 14 or later. Updates are installed manually.\n\n"
        "Download the installer for your platform, or use a ZIP for manual "
        "installation. The preview Flatpak is a sideloaded bundle and does not "
        "receive Flathub updates yet; close-to-tray is disabled in that package. "
        "SHA256SUMS covers every download, and GitHub "
        "records signed build-provenance attestations. iOS archives are "
        "experimental and available through manual workflow runs only.\n\n"
        + android
    )


if __name__ == "__main__":
    version = os.environ["RELEASE_VERSION"]
    prepare(Path("release-assets"), Path("dist"), version)
    Path("release-notes.md").write_text(release_notes(version), encoding="utf-8")
