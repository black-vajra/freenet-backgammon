# Challenge-Capable Lobby Contract Publication

Date: August 28, 2026

## Milestone

The challenge-capable backgammon lobby contract was successfully built, published through one Freenet node, retrieved locally, and independently retrieved from a second Freenet node using a VPN and a different Internet egress path.

The authoritative state retrieved by both nodes was byte-for-byte identical to the canonical initial state generated from the current Rust contract types.

This establishes that the parent lobby contract containing both authenticated presence and challenge state is live and retrievable across independently connected Freenet nodes.

## Source State

Repository:

`~/Desktop/freenet-backgammon`

Workspace:

`experiments/backgammon-protocol-core`

Branch:

`network-actions-0.1`

Source checkpoint:

`d06bdff Add verified client lobby codec`

Full source commit:

`d06bdffc88b821a750fc74a133e69a1cc37fbb85`

The local branch, `origin/network-actions-0.1`, and the actual remote branch were synchronized at this commit before publication. The tracked worktree was clean.

## Contract Scope

The published parent state contains authenticated lobby-presence entries and authenticated challenge state, including bounded challenge retention and synchronization summaries. It retains compatibility handling for legacy presence-only state.

The canonical empty initial state contains an empty player collection and an empty challenge-offer collection.

Canonical CBOR:

`a2656c6f626279a167706c6179657273806a6368616c6c656e676573a1666f666665727380`

State size: 37 bytes

State SHA-256:

`17e99221aa86fc20cd4efc1495950e55d64bd2f7a5eeff8a3301edf737ad6ee7`

## Build Artifact

The contract was built locally using Freenet Development Tool `0.3.287`.

The confirmed build was run from the lobby-contract crate with explicit temporary and workspace target directories:

```text
cd /home/pepper/Desktop/freenet-backgammon/experiments/backgammon-protocol-core/crates/backgammon-lobby-contract
TMPDIR=/home/pepper/quarantine/tmp \
CARGO_TARGET_DIR=/home/pepper/Desktop/freenet-backgammon/experiments/backgammon-protocol-core/target \
fdev build
```

Generated packaged contract:

`crates/backgammon-lobby-contract/build/freenet/backgammon_lobby_contract`

Packaged size: 411,766 bytes

Packaged SHA-256:

`c9c3ae663b5712e7b07b9e275c2bb195d96af19805f116bf73d45b0fbc029876`

WASM SHA-256:

`409c7d3ab7ce31a5dec7cb1528fa8405f26f1e7f4dc10071c8a7cc9d9b8773cb`

Contract API version:

`0.0.1`

Code hash:

`GvWTGD5szVSgGyHexKtSvbUHAwSDWae9bjfZaa5eM9Gc`

Contract parameters:

CBOR null, encoded as `f6`

Parameters SHA-256:

`b0b2988b6bbe724bacda5e9e524736de0bc7dae41c46b4213c50e1d35d4e5f13`

Contract ID:

`CuzYmHzg94LwEpQP9sXTXhHHsAKB6pYC5uABt42CHR8K`

The contract ID was calculated locally before publication and matched the instance key returned by the publishing node.

## Publication and Local Retrieval

Publishing node:

- Host: `pots`
- Freenet version: `0.2.125 (5b8298773494-dirty)`
- Patched release binary with automatic updates disabled
- Freenet Development Tool: `0.3.287`
- Publication requested a node subscription

All artifact hashes, the packaged code, and the locally calculated contract ID were verified immediately before the single network publication attempt.

The publication command returned status 1 because `fdev` classified the successful `UpdateNotification` as an unexpected response. The notification nevertheless contained the exact expected contract ID, code hash, and 37-byte initial state.

No publication retry was performed.

A separate GET through the publishing node then completed with status 0 and retrieved exactly 37 bytes.

Local retrieved-state SHA-256:

`17e99221aa86fc20cd4efc1495950e55d64bd2f7a5eeff8a3301edf737ad6ee7`

The locally retrieved state was byte-for-byte identical to the canonical published seed.

## Independent Cross-Node Retrieval

Independent retrieval node:

- Host: `vulfen`
- User: `sable`
- Separate VPN Internet egress
- Different Freenet peer set from the publishing node

The first retrieval attempt under Freenet `0.2.120` and Freenet Development Tool `0.3.282` failed after no state fragments were received within the stream-assembly inactivity timeout.

The node had 28 connected peers, demonstrating that it had basic network connectivity even though that retrieval attempt failed.

After a verified backup, the independent node was upgraded to:

- Freenet `0.2.131 (9196e4be66c4)`
- Freenet Development Tool `0.3.293`

One controlled service restart placed the updated binary into service. The restarted node became active, re-established a substantial peer set, and then performed one new retrieval attempt.

Cross-node GET status: 0

Retrieved state size: 37 bytes

Cross-node retrieved-state SHA-256:

`17e99221aa86fc20cd4efc1495950e55d64bd2f7a5eeff8a3301edf737ad6ee7`

The independently retrieved state was byte-for-byte identical to both the canonical published seed and the state retrieved through `pots`.

The successful post-upgrade retrieval establishes compatibility with Freenet `0.2.131`. The sequence does not, by itself, prove that Freenet `0.2.120` caused the initial timeout.

## Evidence

Publication artifact directory:

`artifacts/lobby-challenge-publication-20260828-010820`

Important publishing-node logs:

- `lobby-challenge-capable-fdev-build-target-fix-20260828-010406.log`
- `lobby-challenge-seed-and-id-20260828-010820.log`
- `lobby-challenge-network-publication-20260828-012136.log`
- `lobby-challenge-local-retrieval-20260828-012258.log`

Independent-node evidence:

- `backgammon-challenge-lobby-cross-node-v02131-20260828-014924.log`
- `backgammon-challenge-lobby-v02131-20260828-014924.cbor`
- `freenet-update-0.2.120-to-latest-20260828-014222.log`

The independent-node evidence remains preserved on `vulfen`.

## Limitations

This milestone proves:

- Successful publication of the challenge-capable lobby contract.
- Correct code and instance identity.
- Exact local retrieval.
- Exact retrieval through an independently connected Freenet node.
- Compatibility of independent retrieval with Freenet `0.2.131`.

It does not yet prove:

- Browser subscription to this lobby instance.
- Publishing or renewal of authenticated presence.
- Challenge submission or terminal challenge evidence through Freenet.
- Live synchronization of non-empty challenge state.
- Transition from an accepted challenge into game creation.

## Telemetry Observability

At documentation time, the contract ID was not returned by searches in two Freenet network-visualization interfaces. Those interfaces expose observed telemetry rather than an authoritative contract registry. Their search result does not conflict with the successful direct retrieval and exact cross-node hash verification.

## Significance

The challenge-capable shared lobby boundary is now deployed and independently retrievable. The contract is no longer merely compiled source or a local WASM artifact.

The next phase is dedicated browser lobby transport: request and subscribe to this contract, verify authoritative state with the client lobby codec, publish and renew authenticated presence, and project verified available players into the visible lobby.
