# Two-Node Live-Game Findings — 2026-09-13

## Status

Freenet Backgammon has completed two full games between independent Freenet
nodes. Both games ran from initial creation through victory using authenticated
browser identities, the published lobby, and authoritative game contracts.

The sessions validated the Backgammon engine and the fundamental cross-node
protocol. They also exposed synchronization, recovery, matchmaking, and user
interface defects that must be corrected before public distribution. This
document separates observed facts from proposed diagnoses; exact source-level
causes still require verification.

## Test environment

Both machines ran the same locally patched Freenet build:

- Version: `0.2.135 (ea1ff5f169bc-dirty)`
- SHA-256: `6a3be38af467686147140f28b3aceda8005879cbf7643821ddeb120cc90f8586`
- Browser API: `ws://127.0.0.1:7509/v1/contract/command?encodingProtocol=native`

Pots served the client locally with Trunk. The EliteBook received the same
static client through an SSH tunnel to Pots, while each browser connected to
its own local Freenet node on loopback port 7509.

The first complete game linked nodes in Florida and Brazil. The second used
Stockholm and Sao Paulo VPN exit locations. Those exits do not necessarily
describe the complete Freenet routing path.

## Successful results

### Two complete cross-node games

Two games were played from beginning to end. The second ended with the client
correctly displaying `White wins! Single game — 1 point`.

The sessions exercised initial game creation, reciprocal authenticated roles,
fair-roll exchange, ordinary and double rolls, legal-move enforcement, hits,
bar entry, forced pass, bearing off, turn progression, victory detection,
remote propagation, and authoritative recovery after interruption.

No confirmed defect was found in the underlying Backgammon rules during either
complete game.

### Recovery under interruption

The EliteBook lost power during the first game, rebooted, recovered the
authoritative state, and continued. Pots was deliberately rebooted during the
second game and recovered the same board and move history. The EliteBook also
experienced a power interruption during the second game and recovered.

These results provide strong evidence that an established game ledger can
survive browser, client, and host interruption.

### Forced pass and player labels

The forced-pass path worked during live play. When a player had no legal move,
`Pass turn` correctly advanced the authoritative game. During an activated
game, the player cards also identified the local and remote sides as `You` and
`Opponent`; that useful presentation should be retained.

## P0: creation can stall after authenticated genesis exchange

A new accepted game created after the second completed game exposed a
reproducible activation failure.

### Identifiers

- Game ID: `b11f4f44738d89a16ee9d07346858c5201a36de044b1c3530dff9eb268def1cb`
- Contract: `wHeqEj8Ztv9oXubopKLb6MRMURVSCAdEUAKVhT6NJKz`
- Pots player ID: `8534a8ad7e0dfdd82b299646edd1f7d1bc236f51a8f425cd0b2efb684f877e88`
- Elli player ID: `b97f5803b46085817d5862f178d13b34776990160173bfa5363ea3f62ef2baa4`
- Pots role: White
- Elli role: Black

Both clients selected the same ID, displayed the same contract, reported the
game as active, assigned reciprocal local roles, stored their local genesis
shares, and reported:

```text
Both genesis shares confirmed; authenticated genesis ready
```

Both then retrieved a 10-byte contract state, maintained an active
subscription, and reported:

```text
Empty ledger verified — waiting for authenticated game creation
```

Both left `Identity role` at `Waiting for game state`, said that the browser
identity was not yet an authoritative participant, and disabled `Roll`.

Pots additionally displayed:

```text
Freenet connection failed
Freenet WebSocket error: request error:
{"error":"connection closed","source":"close"}
```

Elli continued to report a live connection to its local Freenet node.

Challenge acceptance and authenticated genesis-share exchange therefore
completed, but the authenticated `CreateGame` action did not appear in the
authoritative ledger. The WebSocket closure on Pots is relevant evidence, but
has not been proven to be the root cause.

### Required regression

Add a test that activates the same accepted game for both identities, confirms
both shares, closes the submitting WebSocket between readiness and durable
`CreateGame` confirmation, reconnects, and proves that identical authenticated
creation is recovered exactly once. The contract must advance beyond the empty
ledger, both authoritative roles must resolve, and White's `Roll` control must
be enabled only after authoritative creation.

Source inspection must determine which participant submits creation, how
readiness triggers submission, which durable marker prevents duplicates, how
an uncertain result is reconciled, and whether reconnect re-enters the creation
path reliably.

## P0: authoritative state must not move backward

During the second game, an older state briefly appeared after a newer state had
already been observed. A prior forced-pass position and its dice reappeared
after the pass and a later opponent move had been accepted. The display later
corrected itself. This was not proven to be a new automatic roll; an older
recovery or subscription response is the stronger working explanation.

Once a client has verified history through action N, it must not render a
history shorter than N or one with a divergent prefix. An identical history
may leave the display unchanged. A strict verified extension may advance it.
Older or divergent responses must be quarantined while the newest verified
state remains visible.

Add a deterministic regression delivering verified histories in the order 33,
31, and 34. The rendered state must remain at 33 until it advances atomically
to 34. During uncertain reconciliation, controls should be disabled and the UI
should say `Synchronizing latest verified game state`.

## P0: board, dice, turn, and legal moves must change atomically

Several visible components temporarily represented different authoritative
moments. `Black awaiting fair roll` appeared before the preceding dice
disappeared. A consumed die briefly remained available in move suggestions.
Legal destinations briefly reflected a prior preview, and turn text sometimes
said `White awaiting fair roll` after White had rolled.

Board position, active player, turn phase, current and remaining dice, legal
destinations, bar and bear-off counts, and move history must derive from one
immutable view model identified by verified action count and ledger hash.
Authoritative reconciliation must replace that snapshot atomically.

Local preview must likewise apply a candidate move, consume its die, recompute
legal continuations, and update the board, dice, and highlights together. It
must be labeled as awaiting authoritative confirmation and replaced as one
unit by the next verified state.

## Connection status and recovery

The current `Freenet connected` or `Freenet connection failed` badge conflates
the local WebSocket, individual request results, lobby retrieval and
subscription, game retrieval and subscription, synchronization, and recovery.

During game two, Pots displayed failure while some authoritative changes still
arrived. `Reconnect` subsequently worked and restored a stable green status.
This does not prove the red state was cosmetic; it proves a single badge is
insufficient.

Display separate states for local node connection, lobby subscription, game
contract retrieval, game subscription, latest verified action, and recovery.
Successful later operations must clear stale request errors. A closed socket
should initiate bounded automatic recovery, with manual reconnect available
when that cannot complete.

## Matchmaking and accepted-game selection

Accepted records retained completed and incomplete games. Retention is useful
for audit and recovery, but the current presentation makes the current game
difficult to identify. The newest game appeared in different list positions
during different tests, so position must never imply recency.

The narrow status column also wraps full identifiers almost character by
character and extends far below the viewport.

Replace the raw list with `Current game`, `Awaiting activation`, `Recoverable`,
`Failed activation`, and `Completed archive` sections. Each entry should show
opponent, local color, status, time, shortened game and contract IDs with copy
controls, and the appropriate `Activate`, `Resume`, `Inspect`, or `Archive`
action. The archive should be independently scrollable.

## Rematch and new-game controls

After game two, `Play again` reset the visible board but did not create a new
accepted game or contract. Refreshing returned the browser to an unselected
state while the completed game remained in the accepted-game list.

`New game` displayed a confirmation dialog, but confirming it also did not
create a network game; the selected ID and role remained unchanged. These
controls can therefore display a plausible initial board without proving that
a new authoritative game exists.

Required behavior:

- `Play again` offers a rematch to the same opponent.
- The rematch visibly specifies proposed colors.
- `Switch colors` creates a new accepted contract with reciprocal roles.
- `New opponent` returns to matchmaking.
- `Clear selection` remains a distinct recovery or diagnostic action.
- No control displays a playable board for an old, failed, or nonexistent
  authoritative contract.

Color assignment must be visible before acceptance. Once an accepted contract
fixes the roles, disabling `Play As` controls is appropriate. The exact mapping
between challenger, recipient, White, and Black must be documented and tested.

## Availability and turn wording

A player could participate normally while marked `Unavailable`. The state
controls challenge discovery rather than an established game, but the wording
suggests disconnection. Replace `Available` and `Unavailable` with `Accepting
new challenges` and `Not accepting new challenges`. Show active-game
connectivity separately.

Turn text should identify the precise phase and expected action, for example:

- `White's turn — roll dice`
- `White's roll is awaiting network confirmation`
- `White rolled 3–5 — choose your moves`
- `White rolled 1–2 — no legal move; pass turn`
- `White's move is awaiting network confirmation`
- `Black's turn — waiting for opponent`
- `Synchronizing latest verified game state`

The UI must not instruct a player to roll while `Roll` is disabled because
authoritative creation has not completed.

## Clean testing policy

Published Freenet contracts should not be treated as a deletable local
database. Existing state may be replicated and is useful evidence. Do not wipe
node databases, player identities, secrets, browser identities, or completed
ledgers merely to obtain a clean test environment.

After repair, preserve existing contracts, publish a fresh lobby using the
canonical empty lobby state, retarget the development client, preserve both
identities, verify the lobby independently through both nodes, run the
activation/disconnect regression, and only then attempt another full game.

## Recommended implementation order

1. Trace authenticated-genesis readiness through `CreateGame` submission,
   confirmation, and reconnect recovery.
2. Add the deterministic activation/disconnect regression.
3. Repair exactly-once authenticated game creation.
4. Enforce monotonic verified-ledger rendering.
5. Create one atomic authoritative game-view snapshot.
6. Repair local preview and die-consumption coherence.
7. Separate connection, subscription, request, and synchronization statuses.
8. Redesign rematch and new-game behavior.
9. Redesign accepted-game history and scrolling.
10. Clarify availability, role, turn, and waiting-state language.
11. Publish and verify a fresh empty test lobby.
12. Run a third full two-node acceptance game.

## Overall assessment

Freenet Backgammon is more than a local UI demonstration: two independent
nodes completed two authoritative games, including recovery from host and
power interruption. The rules and established-game ledger performed very
well. Remaining work is concentrated in lifecycle orchestration, high-latency
presentation, reconnect semantics, matchmaking history, and accurately
communicating the difference between local intent and verified authoritative
state.
