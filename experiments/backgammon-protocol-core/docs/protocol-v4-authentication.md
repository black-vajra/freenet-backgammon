# Protocol v4 — Authenticated Game Actions

Status: design contract
Target protocol version: 4

## Purpose

Protocol v4 adds cryptographic authentication to replicated backgammon game
actions.

Protocol v3 already verifies deterministic replay, action ordering, action-ID
uniqueness, the state-hash chain, fair-dice transitions, legal moves, terminal
states, and canonical typed payload encoding. It does not prove that the player
named by an otherwise valid action actually authorized that action.

Protocol v4 closes that trust gap by requiring Ed25519 authentication of every
accepted game action.

This document defines the authentication contract before implementation. The
goal is to avoid repeated incompatible protocol changes while later work adds
reconnection, segmented storage, lobby/challenge handling, and additional user
interface behavior.

## Existing identity model

A player identity is:

    type PlayerId = [u8; 32];

For the existing persistent browser identity:

    PlayerId == Ed25519 verifying-key bytes

No additional hash or identifier transformation is used.

The private Ed25519 signing key remains local to the player. It must never be
stored in the replicated game record, sent to Freenet, passed between players,
or supplied to tooling merely to construct public game state.

## Authentication boundary

Authentication is enforced by `backgammon-protocol`.

The browser is responsible for producing signatures with its local private
key. The protocol library is responsible for reconstructing the exact signed
message and verifying signatures.

The Freenet contract must accept game state only after the protocol
authentication checks and deterministic replay checks succeed.

Authentication is therefore not a user-interface convention and is not trusted
merely because an opponent claims that a record is signed.

## Finalized action body

The signed action body contains exactly the authoritative fields of one game
action:

    protocol_version
    game_id
    action_id
    sequence
    previous_state_hash
    resulting_state_hash
    payload

Authentication data itself is not part of this body.

The resulting-state hash must be derived before signing. Once the
resulting-state hash has been derived, the action body is immutable.

Any change to any signed field after signing must invalidate the signature.

## Signing sequence

A normal action is constructed in this order:

    1. Verify the existing action history.
    2. Determine the next sequence number.
    3. Set the previous-state hash from the verified current state.
    4. Apply the candidate payload to a cloned replay state.
    5. Derive the resulting-state hash.
    6. Construct the finalized immutable action body.
    7. Canonically encode the signing message.
    8. Produce the required Ed25519 signature or signatures.
    9. Construct the authenticated game-action record.
   10. Verify authentication and replay the complete candidate history.

A signature must never be generated over a record whose resulting-state hash is
still a placeholder.

## Domain separation

Game-action signatures use an explicit v4 domain.

The signing message is:

    ASCII("freenet-backgammon/action/v4")
    || 0x00
    || canonical_cbor(finalized_action_body)

The zero byte separates the fixed domain from the encoded action body.

Signatures are produced over these complete bytes directly with Ed25519.

The domain prevents a valid signature created for another purpose, including a
future lobby message or challenge message, from being reused as a game action.

A different protocol message family must use a different signing domain.

## Canonical signing encoding

The signing body is reconstructed from the decoded typed action fields and
encoded locally by `backgammon-protocol`.

No peer-supplied "bytes that were signed" field is trusted.

The same canonical CBOR implementation used by the protocol must produce the
same signing bytes on every implementation that claims v4 compatibility.

Protocol-v4 golden fixtures must freeze the exact canonical signing bytes for
representative actions.

At minimum, fixtures must cover:

- CreateGame;
- RequestRoll;
- CommitDice;
- RevealDice;
- PlayTurn;
- Resign.

Decoding followed by canonical re-encoding must reproduce the normative bytes.

## Authentication data

The replicated action record carries authentication in addition to the
finalized action body.

Semantically, authentication has two forms:

    Genesis:
        white_signature
        black_signature

    PlayerAction:
        signature

Each Ed25519 signature is exactly 64 raw bytes.

The implementation must encode these signatures deterministically. The exact
v4 CBOR representation will be frozen by golden fixtures before a live v4 game
is published.

No private key material is part of the authentication record.

## CreateGame authorization

`CreateGame` requires authorization from both configured players.

White and Black each sign the exact same finalized genesis action body.

Verification therefore requires:

    verify(
        configuration.white.id,
        white_signature,
        signing_message
    )

and:

    verify(
        configuration.black.id,
        black_signature,
        signing_message
    )

Both signatures are required.

A game is not authenticated merely because one player constructed a
configuration containing the other player's public key.

This means the eventual challenge/accept flow must obtain both genesis
signatures before the authenticated game is published.

The existing development CreateGame generator does not possess either player's
private key and must not be changed to accept private identity seeds on a
command line merely to preserve that workflow.

Deterministic test keys may be used in automated fixtures and tests.

## Post-genesis signer rules

For these actions, the `player` named by the typed payload is the required
signer:

    RequestRoll
    CommitDice
    RevealDice
    PlayTurn
    Resign

The required public key is derived from the verified genesis configuration:

    Player::White -> configuration.white.id
    Player::Black -> configuration.black.id

The verifier does not trust a separate peer-supplied public key.

The payload's player field is covered by the signature because the complete
payload is part of the finalized signed action body.

## Abandon is reserved in v4

The current protocol-v3 payload contains:

    Abandon { player }

and replay records:

    ReplayStatus::Abandoned { player }

That representation identifies the player considered abandoned, but it does
not identify who is asserting abandonment or provide deterministic evidence
that an abandonment policy was satisfied.

Therefore protocol v4 must not guess a signer rule for this action.

Until reconnection and abandonment policy is explicitly designed, authenticated
v4 verification must reject `Abandon` as unsupported.

Security must not be weakened merely to avoid a future protocol change. If a
safe abandonment policy can later use the existing v4 representation without
ambiguity, it may be enabled only after deterministic cross-client validation.
If the required wire semantics differ, a later incompatible protocol revision
is preferable to an unsafe shortcut.

## Verification order

For the genesis action:

    1. Verify protocol version.
    2. Verify typed CreateGame payload and GameConfiguration.
    3. Verify genesis sequence and genesis previous-state hash.
    4. Reconstruct canonical signing bytes.
    5. Verify White's signature using configuration.white.id.
    6. Verify Black's signature using configuration.black.id.
    7. Apply the CreateGame transition.
    8. Verify the resulting-state hash.

For each later action:

    1. Verify the already accepted history.
    2. Verify protocol version and typed payload structure.
    3. Verify sequence, action-ID uniqueness, game ID, and previous-state hash.
    4. Determine the required player from the payload.
    5. Resolve that player to a PlayerId from the verified genesis
       configuration.
    6. Reconstruct canonical signing bytes.
    7. Verify the Ed25519 signature.
    8. Apply the action through deterministic replay.
    9. Verify the resulting-state hash.
   10. Accept the action only if every check succeeds.

No unauthenticated action may become part of accepted authoritative state.

## State hashes and signatures are separate

Authentication data is not included in `CanonicalReplayState`.

Authentication data is not included in `resulting_state_hash`.

The state hash answers:

    What deterministic game state results from this accepted history?

The signature answers:

    Did the required player authorize this exact action at this exact
    position in the history?

Keeping these responsibilities separate prevents circular construction and
allows storage architecture to change without changing game-state semantics.

## Action IDs are signed

`action_id` is part of the finalized signed action body.

A valid signature therefore binds the action to its exact ID as well as its
game ID, sequence position, previous-state hash, resulting-state hash, and
payload.

Changing an action ID after signing invalidates authentication.

## Replay protection

A signature is bound to:

- protocol version;
- game ID;
- action ID;
- sequence number;
- previous-state hash;
- resulting-state hash; and
- payload.

An action copied from another game or another position in the same game must
therefore fail authentication or existing replay/hash-chain validation.

Existing duplicate action-ID and sequence checks remain mandatory.

## Wire representation

Protocol v4 changes the replicated `Action` representation because
authentication must survive Freenet storage, retrieval, merge, delta delivery,
and replay.

`Action::from_game_action_record()` and `Action::to_game_action_record()` must
round-trip authentication without modification.

The Freenet contract must verify authenticated typed history after converting
wire actions to typed records.

Protocol-v3 unsigned actions are not valid protocol-v4 actions.

There is no requirement for a v4 contract to accept mixed v3/v4 histories.

## Cryptographic dependency

Verification uses Ed25519 through `ed25519-dalek` 3.0.0.

The intended protocol dependency is verification-only:

    ed25519-dalek = {
        version = "3.0.0",
        default-features = false
    }

A disposable compile probe has already verified this configuration for:

- the native Rust target; and
- `wasm32-unknown-unknown`.

The protocol and Freenet contract do not require random-number generation or
private signing-key generation.

Private-key signing remains a client responsibility.

## Required protocol-v4 tests

Before browser wiring, automated protocol tests must demonstrate at least:

- valid dual-signed CreateGame is accepted;
- CreateGame missing either signature is rejected;
- CreateGame signed by the wrong key is rejected;
- valid signed RequestRoll is accepted;
- valid signed CommitDice is accepted;
- valid signed RevealDice is accepted;
- valid signed PlayTurn is accepted;
- valid signed Resign is accepted;
- unsigned post-genesis action is rejected;
- action signed by the opponent is rejected;
- action signed by a nonparticipant is rejected;
- mutation of game ID after signing is rejected;
- mutation of action ID after signing is rejected;
- mutation of sequence after signing is rejected;
- mutation of previous-state hash after signing is rejected;
- mutation of resulting-state hash after signing is rejected;
- mutation of payload after signing is rejected;
- signature copied from another action is rejected;
- signature copied from another game is rejected;
- malformed signature length is rejected;
- malformed public key is rejected;
- unsupported Abandon is rejected;
- canonical signing encoding matches v4 golden fixtures;
- authenticated history reconstructs the same deterministic state on repeated
  replay.

## Required contract tests

The Freenet contract test path must demonstrate that:

- a correctly authenticated history is accepted;
- a forged signature is rejected during state validation;
- a forged signature is rejected during update application;
- a state containing an unsigned v4 action is rejected;
- delivery grouping or ordering does not bypass authentication;
- state synchronization does not strip or alter signatures; and
- authenticated action records survive exact encode/decode round trips.

The contract must not rely on the browser having already checked signatures.

## Required client behavior

The browser must:

- load the local persistent Ed25519 signing key;
- derive its PlayerId from the corresponding verifying key;
- derive its authoritative role from the verified genesis configuration;
- refuse to sign an action for a role it does not control;
- sign only a fully finalized action body;
- never expose the private identity seed to the opponent or Freenet;
- verify retrieved authoritative history through the shared protocol before
  presenting it as accepted game state.

Browser-generated action IDs and dice secrets remain separate concepts from the
identity signing key.

## Compatibility with later storage work

Protocol-v4 authentication belongs to the action record, not to the current
monolithic ledger container.

A later bounded, hash-chained segmented ledger should therefore be able to move
the same authenticated v4 action records between storage segments without
changing their signatures.

Storage segmentation alone should not require a game-action protocol-version
increment.

Snapshots remain replay accelerators and must not replace authenticated
append-only history as the source of truth.

## Compatibility with lobby and challenges

Lobby announcements, availability records, challenges, acceptances, and
declines are not game actions.

They should use their own message types and signing domains.

Adding those systems must not require changing the authenticated v4 game-action
record merely because the same player identities are reused.

## Security properties claimed by v4

After successful implementation and testing, v4 is intended to establish that:

- every accepted gameplay action was authorized by the required participant;
- both participants authorized the exact genesis game configuration;
- signatures cannot be transplanted to another game or history position;
- mutation of an authenticated action invalidates it;
- the Freenet contract independently enforces authentication;
- deterministic replay remains the authority for game legality and state; and
- possession of replicated game data does not reveal either player's private
  signing key.

These properties must not be claimed until the implementation and contract
tests proving them are complete.

## Explicit non-goals of this change

Protocol-v4 authentication does not itself implement:

- player discovery;
- availability announcements;
- challenges;
- challenge expiration;
- reconnection policy;
- abandonment timing;
- spectators;
- chat;
- rankings;
- tournaments;
- doubling cube rules;
- Crawford rule;
- segmented ledger storage; or
- encrypted game history.

Those systems may build on authenticated PlayerIds without weakening or
redefining the signed game-action body.

## Versioning rule after v4

The game-action protocol version is incremented only for an incompatible change
to replicated game-action semantics or encoding.

User-interface changes, lobby behavior, transport changes, storage-container
changes, and other work that preserves the authenticated v4 action record do
not justify another game-action protocol version.

The objective is to keep v4 stable through the remainder of the playable alpha
unless a genuine incompatible security or protocol requirement makes another
revision necessary.
