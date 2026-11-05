#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
command -v slint-tr-extractor >/dev/null || {
  echo "slint-tr-extractor 1.18.1 is required" >&2
  exit 1
}
command -v msgmerge >/dev/null || {
  echo "GNU gettext is required" >&2
  exit 1
}

work_dir=$(mktemp -d)
trap 'rm -rf -- "$work_dir"' EXIT
cd "$repo_root"

# Preview and screenshot scripts use temporary hidden .slint files.
find ui -type f -name '*.slint' ! -name '.*' -print0 \
  | sort -z \
  | xargs -0 slint-tr-extractor --no-default-translation-context \
      --package-name evanya --package-version 0.1.0 \
      -o "$work_dir/evanya.pot"

# Extraction timestamps are intentionally volatile; compare every semantic
# catalog line while ignoring only that generated header field.
diff -u \
  <(sed '/^"POT-Creation-Date:/d' lang/evanya.pot) \
  <(sed '/^"POT-Creation-Date:/d' "$work_dir/evanya.pot")

for language in da de es fr it ja nl ru tr zh_Hans; do
  msgmerge --quiet --no-fuzzy-matching \
    --output-file "$work_dir/evanya-$language.po" \
    "lang/$language/LC_MESSAGES/evanya.po" "$work_dir/evanya.pot"
  diff -u \
    <(sed '/^"POT-Creation-Date:/d' "lang/$language/LC_MESSAGES/evanya.po") \
    <(sed '/^"POT-Creation-Date:/d' "$work_dir/evanya-$language.po")
done

bash scripts/check-ui-message-catalog.sh
