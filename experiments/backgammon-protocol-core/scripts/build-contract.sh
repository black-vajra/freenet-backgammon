#!/usr/bin/env bash
set -euo pipefail

WORKSPACE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONTRACT="$WORKSPACE/crates/backgammon-contract"

cd "$CONTRACT"

CARGO_TARGET_DIR="$WORKSPACE/target" fdev build

RAW="$WORKSPACE/target/wasm32-unknown-unknown/release/backgammon_contract.wasm"
PACKAGE="$CONTRACT/build/freenet/backgammon_contract"

if [[ ! -f "$RAW" ]]; then
    printf 'Missing raw WASM: %s\n' "$RAW" >&2
    exit 1
fi

if [[ ! -f "$PACKAGE" ]]; then
    printf 'Missing Freenet package: %s\n' "$PACKAGE" >&2
    exit 1
fi

raw_size="$(stat --printf='%s' "$RAW")"
package_size="$(stat --printf='%s' "$PACKAGE")"

if (( package_size - raw_size != 40 )); then
    printf 'Unexpected package header size: %s bytes\n' \
        "$((package_size - raw_size))" >&2
    exit 1
fi

payload="$(mktemp)"
trap 'rm -f "$payload"' EXIT

tail --bytes=+41 "$PACKAGE" > "$payload"

if ! cmp --silent "$RAW" "$payload"; then
    printf 'Package payload does not equal raw WASM\n' >&2
    exit 1
fi

printf 'Contract build verified.\n'
printf 'Raw WASM:       %s (%s bytes)\n' "$RAW" "$raw_size"
printf 'Freenet package: %s (%s bytes)\n' "$PACKAGE" "$package_size"
