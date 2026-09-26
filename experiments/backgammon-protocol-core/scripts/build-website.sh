#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLIENT="$ROOT/crates/backgammon-client"
DELEGATE_WASM="$ROOT/target/wasm32-unknown-unknown/release/backgammon_local_state_delegate.wasm"
ASSET="$CLIENT/assets/backgammon-local-state-delegate.wasm"
OUT="${1:-/tmp/freenet-backgammon-website}"

cd "$ROOT"

echo "== Building local-state delegate =="
cargo build \
  --release \
  --target wasm32-unknown-unknown \
  -p backgammon-local-state-delegate

echo "== Installing delegate website asset =="
mkdir -p "$CLIENT/assets"
cp "$DELEGATE_WASM" "$ASSET"

echo "== Verifying copied delegate =="
sha256sum "$DELEGATE_WASM" "$ASSET"

echo "== Building website =="
rm -rf "$OUT"

cd "$CLIENT"

trunk build \
  --release \
  --dist "$OUT" \
  --public-url "./" \
  index.html

echo "== Making stylesheet usable in the Freenet sandbox =="
python3 - "$OUT/index.html" <<'PY'
from pathlib import Path
import re
import sys

index = Path(sys.argv[1])
html = index.read_text()
links = list(re.finditer(r'<link\b[^>]*\brel="stylesheet"[^>]*>', html))
if len(links) != 1:
    raise SystemExit(f"Expected one generated stylesheet link, found {len(links)}")

link = links[0]
without_sri, count = re.subn(r'\s+integrity="sha384-[A-Za-z0-9+/=]+"', '', link.group(), count=1)
if count != 1:
    raise SystemExit("Generated stylesheet link had no SHA-384 integrity attribute")

index.write_text(html[:link.start()] + without_sri + html[link.end():])
PY

echo "== Verifying packaged delegate =="
sha256sum \
  "$ASSET" \
  "$OUT/backgammon-local-state-delegate.wasm"

echo
echo "Website build complete:"
echo "$OUT"
