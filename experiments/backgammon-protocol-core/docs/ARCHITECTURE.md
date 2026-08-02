# Freenet Backgammon Architecture

## Status

Initial architecture decision for the first playable alpha.

## Core principle

The backgammon rules engine is transport-independent. Freenet communication,
authentication, synchronization, and user-interface code must not determine
whether a move is legal.

## Crates

### backgammon-core

Owns:

- Canonical board representation
- Starting position
- Board invariants
- Legal checker movement
- Bar entry
- Hitting
- Bearing off
- Complete legal turn generation
- Mandatory dice use
- Higher-die rule
- Victory, gammon, and backgammon scoring

Must not depend on:

- Freenet
- Browser APIs
- Network transport
- Cryptographic identity storage
- User-interface state

### backgammon-protocol

Owns:

- Protocol version
- Game and player identifiers
- Action envelope
- Ordered action history
- Previous-state and resulting-state hashes
- Deterministic serialization
- Duplicate, stale, conflicting, and out-of-turn action rejection
- State reconstruction
- Dice commitment, reveal, and verified-roll records

May depend on backgammon-core.

Must not depend directly on browser UI code.

### backgammon-contract

Owns:

- Freenet ContractInterface implementation
- Composable state, summary, and delta behavior
- State and message size limits
- Decoding and encoding at the contract boundary
- Validation of submitted shared histories
- Rejection of malformed or unsupported updates

Depends on backgammon-protocol and backgammon-core.

## Validation layers

1. Board-rule validation:
   Whether checker movement is legal.

2. Turn validation:
   Whether the complete move sequence correctly consumes the available dice.

3. Protocol validation:
   Whether the action is sequential, authentic, current, bounded, and based on
   the agreed prior state.

## Shared-state model

The authoritative record is an append-only action history.

Network messages may arrive late, duplicated, or out of order. Storage and merge
logic may therefore accept an unordered collection temporarily, but a valid game
must reconstruct into one unambiguous sequential history.

Snapshots may be added as performance aids but never replace the action history.

## Trust boundaries

- Display names are untrusted text.
- Network messages are untrusted bytes.
- The opponent is not trusted to enforce rules.
- The UI is not trusted to establish legality.
- Each client independently verifies every accepted action.
- Private identity keys and unrevealed dice secrets never enter shared contract
  state.
- The contract must reject malformed, oversized, conflicting, stale, or
  unsupported state.

## Current proven foundation

The preserved ledger prototype already demonstrates:

- Production Freenet WASM runtime execution
- Composable state integration
- Delta application
- Idempotent duplicate delivery
- Conflicting-ID rejection
- Payload and action-count bounds
- Protocol-version rejection
- Stable CBOR round trips

The prototype orders records by action ID and does not yet enforce its sequence
field. The game protocol must add ordered-history validation without assuming
ordered network delivery.
