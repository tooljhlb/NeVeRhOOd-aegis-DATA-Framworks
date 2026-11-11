#!/usr/bin/env bash
# Run from a reviewed package; installs into a NEW version directory only.
set -euo pipefail
if [[ $# != 1 ]]; then
  echo 'usage: bash install.sh NEW_INSTALL_DIRECTORY' >&2
  exit 2
fi
package=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
expected_platform="$(uname -s)-$(uname -m)"
[[ $(cat "$package/PLATFORM") == "$expected_platform" ]] || { echo 'package platform mismatch' >&2; exit 1; }
# Fixed manifest membership prevents checksums from reading arbitrary paths.
[[ $(wc -l < "$package/SHA256SUMS") == 6 ]] || { echo 'invalid manifest length' >&2; exit 1; }
for name in aegis-data install.sh LICENSE SOURCE_REVISION PLATFORM VERSION; do
  [[ -f "$package/$name" && ! -L "$package/$name" ]] || { echo 'invalid package member' >&2; exit 1; }
  expected=$(sed -n "s/^\\([a-f0-9]\\{64\\}\\)  $name$/\\1/p" "$package/SHA256SUMS")
  [[ $expected =~ ^[a-f0-9]{64}$ ]] || { echo 'invalid checksum entry' >&2; exit 1; }
  actual=$(sha256sum "$package/$name")
  [[ ${actual%% *} == "$expected" ]] || { echo "checksum mismatch: $name" >&2; exit 1; }
done
prefix=$1
mkdir -- "$prefix"
trap 'rm -rf -- "$prefix"' ERR
mkdir -- "$prefix/bin" "$prefix/share"
cp -- "$package/aegis-data" "$prefix/bin/aegis-data"
chmod 755 "$prefix/bin/aegis-data"
cp -- "$package/LICENSE" "$package/SOURCE_REVISION" "$package/PLATFORM" "$package/VERSION" "$package/SHA256SUMS" "$prefix/share/"
"$prefix/bin/aegis-data" --version
trap - ERR
echo "Installed. Run $prefix/bin/aegis-data --help"
