# First Browser-Originated Freenet Action

**Date:** August 5, 2026  
**Branch:** `local-client-0.1`  
**Implementation commit:** `981dec8a602b63f8268be1545821878bc465e08d`  
**Protocol version:** 2

## Milestone

A browser client submitted the first real append-only backgammon action through a live Freenet node. The contract accepted the delta, produced the expected canonical ledger state, and made that state available to an independent Freenet node.

This milestone proves the minimal client-to-contract-to-network loop required before building the general multiplayer action pipeline.

## Contract

Contract instance ID:

`HA2DEihDKpRuFDAszokohNxWXZvmxyhnvbidDFJnHBCK`

The initial contract state was the canonical empty ledger:

- Size: 10 bytes
- SHA-256: `f77ce2b7d981692678ef152017157104c4a722dfe29ffe4d55b99cc658d973e6`
- CBOR: `a1 67 61 63 74 69 6f 6e 73 80`

The sequence-0 `CreateGame` delta produced the canonical one-action ledger:

- Size: 516 bytes
- SHA-256: `7ee9c67e49405e3d44c39536cd4e701852747b14546bb86794a01e959303d764`

The submitted delta and independently generated expected state were byte-for-byte identical for this first append.

## Verified Behavior

1. The browser retrieved the exact 10-byte empty ledger.
2. The browser retained the complete `ContractKey` returned by GET.
3. The browser submitted the pinned sequence-0 delta with `ContractRequest::Update`.
4. The Freenet contract validated and accepted the update.
5. The browser retrieved and verified the resulting 516-byte ledger.
6. An independent CLI GET retrieved the exact expected state.
7. Reloading the browser did not resubmit sequence 0 or alter the ledger.
8. A second Freenet node on a separate machine and distinct external network path retrieved the same 516-byte ledger.
9. After restarting that second Freenet process, it recovered the same state byte-for-byte.
10. Restricting the second node's local API to loopback did not affect contract retrieval.

## Automated Validation

The implementation was validated with:

- 20 `backgammon-client` tests
- 27 `backgammon-contract` tests
- 59 `backgammon-protocol` tests
- Successful Rust documentation tests
- Successful Trunk/WASM browser build

Total milestone test count: 106 automated tests, plus live browser and cross-node verification.

## Browser Reload Safety

The temporary browser probe submits the pinned first action only when the retrieved state is exactly the canonical empty 10-byte ledger.

Once the state contains the verified 516-byte action, a page reload retrieves and verifies it without submitting another sequence-0 action.

This is a temporary diagnostic safeguard, not the final generalized duplicate-action defense.

## Freenet Finding

The tested `fdev execute update` path failed with `missing contract` even after successful GET and republishing of the exact package.

The working browser path preserves the complete contract key returned by GET and uses that key for UPDATE. The browser therefore avoids the incomplete-key behavior encountered in the CLI update path.

This should remain documented as a version-specific Freenet integration limitation until the current CLI behavior is corrected or its intended invocation is confirmed.

## API Binding

The second node initially used Freenet's default dual-stack API bind. Freenet's own source-address checks restrict API access by default, but defense in depth was added with:

`WS_API_ADDRESS=127.0.0.1`

The resulting listeners were:

- `127.0.0.1:7509`
- `[::1]:7509`

Both are loopback-only. Contract retrieval continued to produce the exact expected ledger after the change.

## Not Yet Proven

This milestone does not yet provide:

- A general typed action-submission API
- Player signatures or authenticated identities
- Commit-and-reveal dice
- Concurrent action conflict handling
- Two-way action submission by separate players
- Lobby announcements or challenges
- Complete networked gameplay

## Next Step

Replace the pinned sequence-0 fixture with a reusable browser action pipeline that:

1. Accepts a typed protocol action.
2. Validates its sequence number and previous-state hash.
3. Encodes the canonical CBOR delta.
4. Submits it using the retained complete contract key.
5. Tracks pending, accepted, duplicated, stale, and rejected actions.
6. Retrieves and independently reconstructs the resulting ledger.
7. Exposes transport status without allowing network data to mutate game state directly.

After that pipeline is tested locally, submit a newly generated sequence-1 action and verify it from both independent nodes.
