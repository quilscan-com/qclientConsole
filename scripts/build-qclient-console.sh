#!/usr/bin/env bash
set -euo pipefail

if [[ $# != 2 ]]; then
  echo 'Usage: build-qclient-console.sh <linux-amd64|darwin-arm64> <output-directory>' >&2
  exit 2
fi

platform="$1"
output_directory="$2"
case "$platform:$(uname -s):$(uname -m)" in
  linux-amd64:Linux:x86_64|darwin-arm64:Darwin:arm64) ;;
  *) echo "Unsupported native build platform: $platform ($(uname -s)/$(uname -m))" >&2; exit 1 ;;
esac

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$(bash "$repo_root/scripts/qclient-console-version.sh")"
cd "$repo_root"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$repo_root/target/console-release}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
export QCLIENT_CONSOLE_BUILD_COMMIT="${QCLIENT_CONSOLE_BUILD_COMMIT:-${GITHUB_SHA:-uncommitted}}"
# Build only the shipped executable. Independent tests stay local and are not
# needed by production checkouts of this repository.
cargo_arguments=(build --release --locked
  --manifest-path "$repo_root/crates/quil-client-console/Cargo.toml"
  --bin qclient-console)
if [[ "${CONSOLE_BUILD_OFFLINE:-0}" == 1 ]]; then cargo_arguments+=(--offline); fi
cargo "${cargo_arguments[@]}"

binary="$CARGO_TARGET_DIR/release/qclient-console"
reported_version="$("$binary" --signature-check=false --version)"
if [[ "$reported_version" != "qclient-console $version" ]]; then
  echo "Compiled version does not match source artifact version: $reported_version (expected $version)" >&2
  exit 1
fi

if [[ "$platform" == darwin-arm64 ]]; then
  # The macOS download must work without this runner's Homebrew libraries.
  dependencies="$(otool -L "$binary")"
  while IFS= read -r dependency; do
    case "$dependency" in
      /usr/lib/*|/System/Library/*|'') ;;
      *) echo "Non-system macOS runtime dependency: $dependency" >&2; exit 1 ;;
    esac
  done < <(printf '%s\n' "$dependencies" | tail -n +2 | awk '{print $1}')
fi

mkdir -p "$output_directory"
artifact="$output_directory/qclient-console-$version-$platform"
install -m 0755 "$binary" "$artifact"
printf 'Built %s\n' "$artifact"
