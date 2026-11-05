#!/usr/bin/env bash
# Run only in a disposable Fedora container: installs packages and a test user.
# Usage: bash scripts/test-rpm.sh /absolute/path/package.rpm /results
set -euo pipefail

project_dir="$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)"
package="$(realpath "${1:?RPM path is required}")"
results="$(realpath "${2:?Results directory is required}")"
mkdir -p "$results"
if rpm -q evanya; then
  printf 'Expected a clean container without Evanya installed.\n' >&2
  exit 1
fi

rpm -qip "$package"
rpm -qp --requires "$package" | tee "$results/requires.txt"
# Resolve dependencies before adding any test tools. Build packages and weak
# dependencies must not hide missing runtime requirements.
dnf install -y --setopt=install_weak_deps=False "$package"
rpm -V evanya
test -x /usr/bin/evanya
test -f /usr/lib64/evanya/libpdfium.so
rpm -qa | sort > "$results/installed-packages.txt"

# Exercise dynamically loaded libraries and the installed PDF worker before
# desktop test dependencies can supply libraries missing from our Requires.
dnf install -y --setopt=install_weak_deps=False python3 shadow-utils util-linux
python3 - <<'PY'
import ctypes

for library in (
    "libfontconfig.so.1", "libX11.so.6", "libX11-xcb.so.1",
    "libXcursor.so.1", "libXi.so.6", "libwayland-client.so.0",
    "libxkbcommon.so.0", "libxkbcommon-x11.so.0",
):
    ctypes.CDLL(library)
    print(f"Loaded runtime library: {library}")
PY
useradd --create-home evanya-rpm-test
chown evanya-rpm-test:evanya-rpm-test "$results"
runuser -u evanya-rpm-test -- python3 "$project_dir/scripts/test-pdf-preview.py" \
  /usr/bin/evanya "$results/pdf-preview.png"

dnf install -y --setopt=install_weak_deps=False \
  appstream desktop-file-utils dbus-daemon xorg-x11-server-Xvfb xorg-x11-xauth weston
desktop-file-validate /usr/share/applications/com.evanya.mail.desktop
appstreamcli validate --no-net /usr/share/metainfo/com.evanya.mail.metainfo.xml

# Require an actual rendered frame and a successful exit, rather than treating
# an application that hangs for 15 seconds as a successful launch.
startup=(python3 "$project_dir/scripts/benchmark-startup.py"
  --binary /usr/bin/evanya --rounds 1 --settle-seconds 0 --idle-seconds 1)
runuser -u evanya-rpm-test -- dbus-run-session -- xvfb-run -a \
  "${startup[@]}" --output "$results/x11.json"

runtime_dir="$(mktemp -d)"
chown evanya-rpm-test:evanya-rpm-test "$runtime_dir"
runuser -u evanya-rpm-test -- env XDG_RUNTIME_DIR="$runtime_dir" \
  weston --backend=headless --renderer=pixman --socket=wayland-rpm --idle-time=0 \
  --no-config > "$results/wayland-compositor.log" 2>&1 &
weston_pid=$!
trap 'kill "$weston_pid" 2>/dev/null || true; wait "$weston_pid" 2>/dev/null || true; rm -rf "$runtime_dir"' EXIT
for ((attempt = 0; attempt < 100; attempt++)); do
  if [[ -S "$runtime_dir/wayland-rpm" ]]; then break; fi
  if ! kill -0 "$weston_pid" 2>/dev/null; then
    cat "$results/wayland-compositor.log" >&2
    exit 1
  fi
  sleep 0.1
done
test -S "$runtime_dir/wayland-rpm"
runuser -u evanya-rpm-test -- env -u DISPLAY \
  XDG_RUNTIME_DIR="$runtime_dir" WAYLAND_DISPLAY=wayland-rpm \
  dbus-run-session -- "${startup[@]}" --output "$results/wayland.json"

dnf remove -y evanya
if rpm -q evanya; then exit 1; fi
test ! -e /usr/bin/evanya
test ! -e /usr/lib64/evanya
test ! -e /usr/share/applications/com.evanya.mail.desktop
test ! -e /usr/share/metainfo/com.evanya.mail.metainfo.xml
printf 'RPM installation, PDF rendering, X11/Wayland startup and removal passed.\n'
