# Canonical replay-state v1 fixtures

These files freeze the exact protocol-v1 replay-state representation:

- `canonical-replay-state-v1.cbor` is the exact serialized CBOR byte sequence.
- `canonical-replay-state-v1.blake3` is the raw 32-byte BLAKE3 digest produced
  from the protocol domain separator followed by those CBOR bytes.

Do not regenerate these files merely because a serialization dependency or
data model changes. A changed fixture means a protocol-level compatibility
change and requires explicit review and, normally, a new protocol version.
