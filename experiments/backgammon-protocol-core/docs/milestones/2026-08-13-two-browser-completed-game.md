# Two-Browser Freenet Game Completion Milestone

Date: 2026-08-13

## Result

A real two-browser Freenet-backed backgammon game was recovered from the
previous 256-action contract limit, continued through the same authoritative
game configuration and fair-dice sequence, and completed normally.

Both browser clients independently displayed:

- White wins
- Single game
- 1 point

The final authoritative ledger was retrieved twice from Freenet and the two
copies were byte-for-byte identical.

## Contract migration

Original contract ID:

    DXLWMDteEwcwhSij6XzXcJX5pEb6jidWtPXH5qCgJMzb

Replacement contract ID:

    22QtWgtvYKxo18vVcYbkTBFvC9GcgntPGigPTzCntrra

The original game stopped when the monolithic ledger reached the temporary
contract limit:

    const MAX_ACTIONS: usize = 256;

For the alpha recovery test this was raised to:

    const MAX_ACTIONS: usize = 2048;

Regression coverage was added to prove that the ledger can cross the former
256-action boundary while still rejecting action 2049.

The exact preserved 256-action state was validated against the updated
contract, used as the initial state of a replacement contract, and retrieved
again from Freenet to verify that the replacement state was byte-for-byte
identical to the interrupted state.

## Fair-dice recovery boundary

The interrupted fair-dice round was preserved at:

    253 RequestRoll turn=42 player=White
    254 CommitDice turn=42 player=White
    255 CommitDice turn=42 player=Black
    256 RevealDice turn=42 player=White

Contract-scoped fair-dice secrets were migrated in both browser contexts.
The stale pending-action record was deliberately not copied.

The replacement game then continued with:

    257 RevealDice turn=42 player=Black
    258 PlayTurn turn=42 player=White

This demonstrated continuation across the former 256-action ceiling without
restarting the game or replacing its agreed history.

## Completed authoritative state

Game directory:

    live-fresh-identity-game-20260813-201255

Primary completed-state capture:

    live-fresh-identity-game-20260813-201255/completed-game-20260813-220839.cbor

Second confirming GET:

    live-fresh-identity-game-20260813-201255/completed-game-second-get-20260813-220839.cbor

Both files:

    139886 bytes

SHA-256:

    a04a7b71dafd7df3625d2d1542be8184a7904ddccdccb01d7fc4cebd3be31ca9

The winning action was:

    270 PlayTurn turn=44 player=White

The recovered game therefore appended 15 actions beyond the preserved
256-action state and finished with 271 total actions.

## Independent replay verification

The completed authoritative ledger was independently replayed through the
protocol inspector.

Verified result:

    bytes=139886
    actions=271
    REPLAY VERIFIED
    next_sequence=271
    next_turn=45
    active_player=White
    turn_phase=AwaitingRoll
    dice=None
    roll_requested_by=None

The inspector output initially appeared unusual because it did not print
`replay.state.status` or `replay.status`.

Inspection of the rules and replay code confirmed that this is not a state
error. When the final checker is borne off, the core sets:

    GameStatus::Completed { winner: White, points: 1 }

`apply_turn_sequence()` then clears the dice and normalizes the turn phase to
`AwaitingRoll`, but it changes `active_player` only if the game remains
`InProgress`. Therefore White correctly remains the recorded active player
after White's winning move.

The replay layer converts the completed board state to
`ReplayStatus::Completed`, and subsequent actions are rejected by the
terminal-state guard.

## What this milestone proves

This test demonstrated, with two independent browser clients and a real
Freenet-backed authoritative state, that the current alpha can:

- maintain matching multiplayer game state across two browsers;
- execute the commit-and-reveal fair-dice sequence;
- recover an interrupted fair-dice round after contract migration;
- resume from an exact preserved authoritative ledger;
- append actions beyond the former 256-action limit;
- enforce and replay legal backgammon turns deterministically;
- preserve a completed game through repeated Freenet retrieval;
- independently reconstruct the complete 271-action history; and
- reach and present a normal scored victory.

## Known limitations

The 2048-action ceiling is an alpha workaround, not the intended production
architecture. The completed 271-action monolithic ledger is already 139886
bytes. Production history should move to bounded, hash-chained append-only
segments, with snapshots used only as replay accelerators.

Protocol version 3 also does not yet cryptographically authenticate individual
game actions. Persistent Ed25519 browser identities and authoritative
`PlayerId` role derivation exist, but hostile remote users are not yet prevented
from forging otherwise valid action records. Signed action authentication is
therefore a required future protocol change.

