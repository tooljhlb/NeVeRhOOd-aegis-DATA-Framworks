#!/usr/bin/env bash
# Package an already-built Linux runtime; no source checkout needed at install time.
set -euo pipefail
if [[ $# != 3 ]]; then
  echo 'usage: package.sh BINARY NEW_PACKAGE_DIRECTORY SOURCE_REVISION' >&2
  exit 2
fi
binary=$(realpath "$1")
out=$2
revision=$3
[[ $revision =~ ^[a-f0-9]{40}$ ]] || { echo 'revision must be a full Git SHA' >&2; exit 2; }
[[ $(uname -s) == Linux ]] || { echo 'Linux packaging only' >&2; exit 2; }
mkdir "$out"
cp "$binary" "$out/aegis-data"
chmod 755 "$out/aegis-data"
cp scripts/install.sh "$out/install.sh"
cp LICENSE "$out/LICENSE"
printf '%s\n' "$revision" > "$out/SOURCE_REVISION"
printf '%s-%s\n' "$(uname -s)" "$(uname -m)" > "$out/PLATFORM"
"$out/aegis-data" --version > "$out/VERSION"
(cd "$out" && sha256sum aegis-data install.sh LICENSE SOURCE_REVISION PLATFORM VERSION > SHA256SUMS)
echo "Package created: $out"
