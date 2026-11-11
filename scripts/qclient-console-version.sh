#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="$(sed -n 's/^pub const VERSION_STRING: &str = "\([^"]*\)";$/\1/p' "$repo_root/crates/quil-config/src/version.rs")"
if [[ ! "$version" =~ ^[0-9]+(\.[0-9]+){2,3}$ ]]; then
  echo 'Invalid source VERSION_STRING; expected three or four numeric parts.' >&2
  exit 1
fi

if [[ -f "$repo_root/subpatch" ]]; then
  subpatch="$(cat "$repo_root/subpatch")"
  if [[ ! "$subpatch" =~ ^[[:space:]]*([0-9]+)[[:space:]]*$ ]]; then
    echo 'Invalid source subpatch; expected one numeric counter.' >&2
    exit 1
  fi
  version="$version.${BASH_REMATCH[1]}"
fi
printf '%s\n' "$version"
