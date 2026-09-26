#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLIENT="$ROOT/crates/backgammon-client"
DELEGATE_WASM="$ROOT/crates/backgammon-local-state-delegate/pinned/identity-v1.wasm"
DELEGATE_SHA256="d16d376df4f64d8b127b0bdab68eecb24b3d0a1c811202ead560312d17cdf87c"
ASSET="$CLIENT/assets/backgammon-local-state-delegate.wasm"
PROFILE_WASM="$ROOT/crates/backgammon-profile-delegate/pinned/profile-v1.wasm"
PROFILE_SHA256="d292f0ba6b1259fad0557a5cfbc9f075eff2570e31a0c90a9dedd57b5a3dc1a8"
PROFILE_ASSET="$CLIENT/assets/backgammon-profile-delegate.wasm"
OUT="${1:-/tmp/freenet-backgammon-website}"

cd "$ROOT"

# The delegate key depends on its exact WASM bytes. Rebuilding it could
# disconnect existing users from their persisted signing identities.
echo "== Verifying pinned identity delegate =="
printf '%s  %s\n' "$DELEGATE_SHA256" "$DELEGATE_WASM" | sha256sum --check

echo "== Installing delegate website asset =="
mkdir -p "$CLIENT/assets"
cp "$DELEGATE_WASM" "$ASSET"

echo "== Verifying copied delegate =="
sha256sum "$DELEGATE_WASM" "$ASSET"

echo "== Verifying pinned profile delegate =="
printf '%s  %s\n' "$PROFILE_SHA256" "$PROFILE_WASM" | sha256sum --check
cp "$PROFILE_WASM" "$PROFILE_ASSET"

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

echo "== Verifying packaged profile delegate =="
sha256sum "$PROFILE_ASSET" "$OUT/backgammon-profile-delegate.wasm"

echo
echo "Website build complete:"
echo "$OUT"
