# Freenet backgammon ledger prototype

This is an isolated Day 2 laboratory contract. It is not the backgammon rules
engine and must not be published yet.

It tests the Freenet synchronization primitive we need: a bounded,
order-independent action set with canonical ordering, idempotent duplicates,
conflicting-ID rejection, CBOR state, and current Freenet contract entry
points.

## Local verification

```bash
cargo test
cargo build --release --target wasm32-unknown-unknown
find target/wasm32-unknown-unknown/release -maxdepth 1 \
  -type f -name '*.wasm' -printf '%f  %s bytes\n'
```

These commands compile and test locally. Do not run `fdev build` or
`fdev publish` for this prototype.
