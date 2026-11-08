#!/usr/bin/env bash
set -euo pipefail

project_dir="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
manifest="$project_dir/packaging/flatpak/com.evanya.mail.yml"
build_root="$project_dir/target/flatpak"
build_dir="$build_root/build"
repo_dir="$build_root/repo"
output="$build_root/evanya.flatpak"

for command in flatpak flatpak-builder; do
  command -v "$command" >/dev/null || {
    printf 'Required tool is missing: %s\n' "$command" >&2
    exit 1
  }
done

rm -rf "$build_dir" "$repo_dir" "$output"
mkdir -p "$build_root"
builder_args=(
  --force-clean
  --disable-rofiles-fuse
  --user
  --assumeyes
  --repo="$repo_dir"
  --install-deps-from=flathub
)
if [[ -n "${SOURCE_DATE_EPOCH:-}" ]] && flatpak-builder --help | grep -- --override-source-date-epoch >/dev/null; then
  builder_args+=(--override-source-date-epoch="$SOURCE_DATE_EPOCH")
fi

# The sources come from crates.io and GitHub, which now and then answer 502,
# 503 or 504, and flatpak-builder does not try again: one bad answer failed
# the whole build ("Failed to download sources"). Fetch them first, with new
# attempts; the build then finds them in the download cache.
attempts=5
for attempt in $(seq 1 "$attempts"); do
  if flatpak-builder --download-only --disable-rofiles-fuse --user "$build_dir" "$manifest"; then
    break
  fi
  if (( attempt == attempts )); then
    printf 'Could not download the Flatpak sources in %s attempts.\n' "$attempts" >&2
    exit 1
  fi
  printf 'Downloading the Flatpak sources failed (attempt %s of %s); trying again.\n' "$attempt" "$attempts" >&2
  sleep $((attempt * 20))
done

flatpak-builder "${builder_args[@]}" "$build_dir" "$manifest"
flatpak build-bundle \
  --runtime-repo=https://flathub.org/repo/flathub.flatpakrepo \
  "$repo_dir" "$output" com.evanya.mail
test -s "$output"
printf 'Built %s\n' "$output"
