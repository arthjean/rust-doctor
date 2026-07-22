#!/usr/bin/env bash
set -euo pipefail

[[ "$#" -eq 4 ]] || { echo "usage: package-native.sh BINARY TARGET FORMAT OUTPUT" >&2; exit 2; }

BINARY=$1
TARGET=$2
FORMAT=$3
OUTPUT=$4
[[ -f "$BINARY" ]] || { echo "binary does not exist: $BINARY" >&2; exit 2; }
[[ "$FORMAT" == tar.gz || "$FORMAT" == zip ]] || { echo "unsupported archive format: $FORMAT" >&2; exit 2; }

mkdir -p "$OUTPUT"
OUTPUT=$(realpath "$OUTPUT")
STAGING=$(mktemp -d)
trap 'rm -rf "$STAGING"' EXIT

BINARY_NAME=$(basename "$BINARY")
cp "$BINARY" "$STAGING/$BINARY_NAME"
touch -t 198001010000 "$STAGING/$BINARY_NAME"

if [[ "$FORMAT" == tar.gz ]]; then
  tar --format=ustar -C "$STAGING" -cf - "$BINARY_NAME" | gzip -n > "$OUTPUT/rust-doctor-${TARGET}.tar.gz"
else
  (
    cd "$STAGING"
    7z a "$OUTPUT/rust-doctor-${TARGET}.zip" "$BINARY_NAME"
  )
fi
