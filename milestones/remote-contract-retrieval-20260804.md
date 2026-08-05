# Independent Freenet Contract Retrieval

Date: 2026-08-04
Branch: `local-client-0.1`

## Contract

Contract instance ID:

```text
HA2DEihDKpRuFDAszokohNxWXZvmxyhnvbidDFJnHBCK
```

Expected canonical empty-ledger state:

```text
a1 67 61 63 74 69 6f 6e 73 80
```

Decoded CBOR meaning:

```json
{"actions":[]}
```

Expected state length: 10 bytes

Expected SHA-256:

```text
f77ce2b7d981692678ef152017157104c4a722dfe29ffe4d55b99cc658d973e6
```

## Local verification

The publishing node retrieved the contract by instance ID using:

```bash
fdev network -p 7509 execute get \
  --timeout 300 \
  --output retrieved-state.cbor \
  HA2DEihDKpRuFDAszokohNxWXZvmxyhnvbidDFJnHBCK
```

Observed result:

```text
Contract HA2DEihDKpRuFDAszokohNxWXZvmxyhnvbidDFJnHBCK: 10 bytes
State written to retrieved-state.cbor
```

The browser client also retrieved the same 10-byte state, verified the empty
ledger, established a subscription, and successfully reconnected after the
local Freenet service was stopped and restarted.

## Independent remote retrieval

A remote Freenet user, Skandragon, reported retrieving the contract state from
an independent node and supplied this hexadecimal dump:

```text
00000000  a1 67 61 63 74 69 6f 6e  73 80  |.gactions.|
0000000a
```

The reported bytes match the expected canonical 10-byte empty-ledger state
byte-for-byte.

## What this demonstrates

- The contract instance was retrievable by exact ID from the publishing node.
- The locally returned state matched the expected canonical empty ledger.
- A separate Freenet participant reported retrieving the identical state.
- The contract was not confined solely to the publishing node's browser
  process or local development server.
- Independent network retrieval succeeded for at least one remote node.

## Limits of the evidence

- The remote command transcript and SHA-256 output were not preserved in this
  repository; the retained remote evidence is the supplied hexadecimal dump.
- This does not prove indefinite persistence, universal availability, or
  retrieval from every Freenet node.
- The published state is still only the empty prototype ledger.
- Networked moves, identities, challenges, fair dice, and two-player gameplay
  have not yet been connected through this contract.
