# Genesis-Share-Capable Lobby Contract Publication

Date: September 11, 2026

## Milestone

The genesis-share-capable backgammon lobby contract was built from the
synchronized `network-actions-0.1` branch, published through the Freenet
node on `pots`, and retrieved separately through that node.

The retrieved authoritative state was byte-for-byte identical to the
canonical 37-byte empty lobby seed.

This deployment replaces the browser's previous challenge-only lobby
target. The earlier immutable lobby remains preserved and retrievable,
but the browser now targets the contract capable of carrying convergent
White and Black genesis-signature-share evidence.

## Published Source State

Repository:

`~/Desktop/freenet-backgammon`

Workspace:

`experiments/backgammon-protocol-core`

Branch:

`network-actions-0.1`

Published source checkpoint:

`06aed55 Add genesis share publication planner`

Full source commit:

`06aed55a398d1fbbf0ce09898c3ed18585ddc0ba`

Local HEAD and `origin/network-actions-0.1` matched exactly before the
build and publication. There were no tracked, staged, or unrelated
untracked changes.

## Contract Capability

The lobby contract retains authenticated presence, challenge offers,
terminal challenge evidence, bounded retention, summaries, deltas, and
convergent state merging.

It additionally accepts, verifies, retains, summarizes, synchronizes,
and convergently merges:

- `WhiteGenesisShare`
- `BlackGenesisShare`

Each share must authenticate the exact accepted genesis proposal and
the player assigned to that role. Genesis-share evidence without an
authenticated challenge acceptance is rejected.

## Validation Before Publication

The lobby-contract test suite passed:

- 13 tests passed
- 0 tests failed

The canonical empty lobby state remained unchanged because the new
evidence variants affect populated accepted-challenge records rather
than the empty lobby representation.

Canonical initial-state CBOR:

`a2656c6f626279a167706c6179657273806a6368616c6c656e676573a1666f666665727380`

Initial-state size:

`37 bytes`

Initial-state SHA-256:

`17e99221aa86fc20cd4efc1495950e55d64bd2f7a5eeff8a3301edf737ad6ee7`

Contract parameters:

CBOR null, encoded as `f6`

Parameters SHA-256:

`b0b2988b6bbe724bacda5e9e524736de0bc7dae41c46b4213c50e1d35d4e5f13`

## Build Artifact

Freenet Development Tool:

`0.3.287`

Contract API version:

`0.0.1`

Packaged contract size:

`423,043 bytes`

Packaged contract SHA-256:

`6609a2a51b7a2ec02b7a4775f599622353397416fe6c2770d001f6297c980806`

Raw WASM SHA-256:

`3bb6c833322c0de03a297419cf2dd01ec31083e9ef13f23ec04f1476cff79ca3`

The package contained the expected WASM magic `0061736d` at byte
offset 40.

Code hash:

`31MDRRmFuWWcbDJDZDXTGejwja1B7FkR3Fpj8LVWeen4`

## Immutable Contract Identity

The contract ID was calculated locally before publication using the
verified package and parameter file:

`EsQreoxJF78Eb3uqAFHyZJtXuzN1hSi7GXycAYvtv9jX`

The publishing node returned the same instance ID and code hash.

Previous challenge-only lobby contract:

`CuzYmHzg94LwEpQP9sXTXhHHsAKB6pYC5uABt42CHR8K`

The previous immutable contract was not modified or removed.

## Publication

Publishing node:

- Host: `pots`
- Freenet binary: patched `0.2.125`
- Freenet service state: active and running
- Freenet Development Tool: `0.3.287`
- Node subscription requested
- Publication timestamp: September 11, 2026 at 23:59:16 UTC

Exactly one publication attempt was performed.

`fdev` returned status 1 because it classified the successful
`UpdateNotification` as an unexpected contract response. The
notification nevertheless contained:

- the exact expected contract instance ID;
- the exact expected code hash;
- the exact canonical 37-byte initial state.

No publication retry was performed.

## Separate Local Retrieval

A separate GET was issued for:

`EsQreoxJF78Eb3uqAFHyZJtXuzN1hSi7GXycAYvtv9jX`

GET status:

`0`

Retrieved size:

`37 bytes`

Retrieved-state SHA-256:

`17e99221aa86fc20cd4efc1495950e55d64bd2f7a5eeff8a3301edf737ad6ee7`

The retrieved state compared byte-for-byte equal to the canonical
published seed.

## Preserved Evidence

Build and publication evidence:

`/mnt/two/backups/freenet/genesis-share-lobby-build-20260911-195202`

Preserved files include:

- canonical initial state;
- contract parameters;
- packaged contract;
- extracted raw WASM;
- artifact checksums;
- build log;
- publication output;
- retrieval output;
- retrieved state;
- combined publication and retrieval log.

## Browser Binding

The browser lobby transport now targets:

`EsQreoxJF78Eb3uqAFHyZJtXuzN1hSi7GXycAYvtv9jX`

This binding is necessary before the client may publish genesis-share
evidence. Publishing that evidence to the older challenge-only contract
would be invalid.

## Remaining Validation

This milestone proves exact build identity, one accepted publication,
node subscription response, and exact local retrieval.

Independent cross-node retrieval remains to be performed. Until then,
this milestone does not claim independent-network-path retrieval.

The client must still invoke the genesis-share publication planner,
retry until each local share is authoritative, import the peer share
into the durable handshake, assemble the dual-signed `CreateGame`
action, submit it to the selected game contract, and confirm the
authoritative resulting game state.
