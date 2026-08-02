# Freenet Backgammon Build Log

This file records development milestones, tests, decisions, unresolved questions,
and Freenet limitations discovered while building the application.

## Project Goal

Build a decentralized multiplayer backgammon application on Freenet where two
independent users can:

- Discover and challenge one another.
- Play a complete rules-enforced game.
- Generate mutually verifiable fair dice.
- Maintain matching authenticated game histories.
- Reconnect and reconstruct the latest agreed position.
- Complete a game without trusting a centralized operator.

## Development Plan

Development is organized into approximately one-hour daily sessions:

1. Confirm the development environment.
2. Study and test the current Freenet application workflow.
3. Design the application architecture and data protocol.
4. Build and test the transport-independent backgammon engine.
5. Build the graphical board and local two-player controller.
6. Add cryptographic identities, signed history, and fair dice.
7. Implement the Freenet lobby, challenges, and game synchronization.
8. Conduct two-client testing and publish a limited alpha.

---

## 2026-08-01 — Day 1: Development Inventory

### Completed

- Inspected the operating system and development environment.
- Confirmed the machine is suitable for development.
- Confirmed the Freenet node is installed and running as a systemd service.
- Confirmed the Freenet browser interface listens only on loopback addresses.
- Confirmed `fdev` is installed.
- Confirmed GitHub CLI authentication for the `black-vajra` account.
- Created the local Git repository.
- Created the private GitHub repository:
  `black-vajra/freenet-backgammon`
- Added and pushed the initial `README.md`.

### Verified Environment

- Operating system: Ubuntu 24.04.4 LTS
- Architecture: x86_64
- Kernel: 7.0.0-28-generic
- Freenet: 0.2.116
- Freenet Development Tool (`fdev`): 0.3.278
- Freenet UI: `127.0.0.1:7509` and `[::1]:7509`
- Git: 2.43.0
- `jq`: 1.7
- `curl`: 8.5.0

### Initial Missing Tools

The first inventory found these tools absent:

- Rust compiler
- Cargo
- Rustup
- `wasm32-unknown-unknown` Rust target
- `cargo-make`
- Ripgrep
- Node.js and npm
- `wasm-pack`
- `wasm-opt`

The official introductory Freenet workflow requires Rust, the WebAssembly target,
and `cargo-make`. Node.js, `wasm-pack`, and `wasm-opt` were not established as
requirements for the initial tutorial.

### Security and Operational Notes

- The Freenet interface is not exposed on an external network interface.
- The node was active and remained within its configured 2 GB memory limit.
- Repeated `RATE LIMIT per-callsite` entries were observed in the journal.
- These messages indicate suppression of repetitive log messages; they do not,
  by themselves, demonstrate discarded network traffic or failed contracts.

### Architecture Direction

The provisional application structure is:

- `game-core`: transport-independent backgammon rules
- `protocol`: actions, serialization, hashes, and signatures
- `contract`: Freenet shared-state validation
- `delegate`: private identity and secret management
- `ui`: lobby, board, game controls, and status displays
- `tests`: rules, protocol, malformed input, reconstruction, and two-client tests

A shared Rust rules core is presently preferred because it may allow the contract
and browser interface to apply the same rules without maintaining two independent
implementations. This remains a provisional decision until the current Freenet
tutorial and browser integration have been tested.

### Tested

- `freenet --version`
- `fdev --version`
- Freenet systemd service status
- Local listening sockets
- Git repository initialization
- GitHub CLI authentication
- Initial commit and remote push

### Uncertainties

- The current Freenet contract and browser application structure still needs to
  be verified through the official tutorial.
- Browser use of a shared Rust/WebAssembly rules engine has not yet been proven.
- Freenet update latency and suitability for responsive live play remain untested.
- The correct lobby and per-game contract organization remains undecided.

### Next Session

Day 2: follow the current official Freenet application tutorial and document:

- Required project structure
- Contract and delegate roles
- Build commands
- Local execution workflow
- Browser-to-node communication
- Publication workflow
- Any discrepancies between the documentation and installed tool versions

---

## Log Maintenance Format

Add one entry after every development session containing:

- **Completed**
- **Decisions**
- **Tested**
- **What works**
- **What remains uncertain**
- **Freenet limitations discovered**
- **Next session**

## 2026-08-02 — Day 1 Completion: Toolchain Installed

### Completed

- Installed Rust through `rustup`.
- Installed the WebAssembly compilation target.
- Installed `cargo-make`.
- Installed Ripgrep.
- Verified the complete development environment.
- Confirmed the local repository is synchronized with GitHub.

### Verified Toolchain

- Rust: 1.97.1
- Cargo: 1.97.1
- Rust toolchain: stable-x86_64-unknown-linux-gnu
- Installed Rust targets:
  - wasm32-unknown-unknown
  - x86_64-unknown-linux-gnu
- cargo-make: 0.37.24
- Ripgrep: 14.1.0
- Freenet: 0.2.116
- fdev: 0.3.278

### Repository State

- Branch: main
- Remote: origin/main
- Working tree was clean before this log update.

### What Works

The machine now has the required foundation for compiling Rust contracts to
WebAssembly, running workspace tests, building Freenet application components,
and publishing through the installed Freenet development tool.

### Next Session

Day 2: verify the current Freenet application workflow and the exact commands
supported by the installed versions of Freenet and fdev.

---

## 2026-08-02 — Ledger Contract Prototype — Milestones 2 and 3

### Completed

- Built a composable Freenet contract prototype for an append-only action ledger.
- Added deterministic CBOR serialization and validation.
- Added protocol-version, payload-size, and total-ledger-size limits.
- Preserved Milestones 2 and 3 as separate repository snapshots.
- Created the repository's authoritative `SHA256SUMS` file.

### Milestone 2

Milestone 2 established the initial convergent ledger behavior.

Tests verified:

- Duplicate actions are idempotent.
- Reusing an action ID with different content is rejected.
- Opposite update orders converge to the same state.

Result: 3 tests passed and the Freenet WASM contract built successfully.

### Milestone 3

Milestone 3 retained the original behavior and added defensive coverage for:

- Malformed CBOR rejection.
- Unsupported protocol-version rejection.
- Oversized action-payload rejection.
- Oversized ledger rejection.
- Noncanonical state rejection.
- Full-state merge convergence.
- Stable CBOR round trips.

Result: 10 tests passed and the Freenet WASM contract built successfully.

### Reproducibility

The contract was rebuilt from the repository copy using an external Cargo target
directory. The rebuilt WASM exactly matched the stored Milestone 3 artifact:

`095a4208daa63459977efb7eeec0905925f65beda9b6da3e07d6706372ee36fb`

Milestone snapshots are recorded in the root `SHA256SUMS` file.

### What Works

The prototype provides deterministic, convergent handling of ordered action
records and rejects several classes of malformed, conflicting, noncanonical,
or oversized state.

### What Remains Uncertain

- Unsupported Freenet `UpdateData` variants have not been exercised through the
  exported contract ABI.
- The WASM has not yet been executed through the live Freenet contract runtime.
- Authentication, game-specific action validation, and fair dice are not yet
  implemented.
- Freenet delivery latency and conflict behavior between two independent clients
  remain untested.

### Freenet Limitations Discovered

No runtime limitation has yet been established. Successful native tests and WASM
compilation do not prove correct behavior when executed by a Freenet node.

### Next Session

Build a runtime-facing harness for the exported contract interface. Verify state
validation, update handling, unsupported update variants, malformed serialized
input, and contract execution before any publication.
---

## 2026-08-02 — Protocol Core 0.1: Complete Rules, Verified State, and Fair Dice

### Completed

- Created a three-crate Rust workspace:
  - `backgammon-core`
  - `backgammon-protocol`
  - `backgammon-contract`
- Implemented a transport-independent backgammon rules engine.
- Implemented typed, versioned game-action records.
- Implemented deterministic replay from an append-only action history.
- Implemented canonical CBOR state serialization and BLAKE3 state hashing.
- Preserved versioned golden fixtures for canonical replay-state encoding and hashing.
- Integrated typed game replay into the Freenet contract.
- Implemented contract-state validation, ordered action synchronization, and delta exchange.
- Implemented cryptographic commit-and-reveal dice generation.
- Removed the transitional trusted-roll action from protocol version 2.
- Created the annotated `protocol-core-v0.1` checkpoint.
- Created the `local-client-0.1` branch for graphical-client development.

### Rules Engine

The rules engine now enforces:

- Standard starting positions.
- Player orientation and turn order.
- Legal checker movement.
- Blocked points.
- Hitting blots.
- Mandatory entry from the bar.
- Use of both dice when possible.
- The higher-die rule when only one die can be played.
- Doubles and partial doubles when later moves become blocked.
- Complete-turn generation and validation.
- Bearing off.
- Oversize-die bearing-off restrictions.
- Gammon and backgammon scoring.
- Game completion.
- Rejection of incomplete, illegal, or out-of-turn move sequences.

The interface and network layers will consume this engine rather than duplicating
or independently deciding the game rules.

### Protocol and State Verification

Each game action includes:

- Protocol version.
- Game identifier.
- Unique action identifier.
- Sequential action number.
- Previous canonical state hash.
- Resulting canonical state hash.
- Typed CBOR action payload.

Both complete histories and received deltas are validated by deterministic
replay. The implementation rejects:

- Missing or duplicate sequence numbers.
- Mixed game identifiers.
- Duplicate action identifiers.
- Reused identifiers with different content.
- Broken state-hash chains.
- Forged resulting-state hashes.
- Unsupported protocol versions.
- Malformed typed payloads.
- Illegal game actions.
- Actions submitted after game completion or resignation.
- Noncanonical, oversized, or malformed contract state.

### Verifiable Fair Dice

Protocol version 2 replaces privately supplied dice with a commit-and-reveal
process:

1. White commits secret random material.
2. Black commits secret random material.
3. White reveals its secret.
4. Black reveals its secret.
5. Both commitments are verified.
6. The secrets are combined with the game ID, turn number, and fixed player ordering.
7. Rejection sampling converts the resulting cryptographic digest into two unbiased dice values.

Both clients can independently reproduce and verify the roll.

The protocol rejects:

- Reveals before both commitments exist.
- Duplicate commitments.
- Duplicate reveals.
- Secrets that do not match their commitments.
- Commitments for the wrong turn.
- Fair-dice actions submitted after a roll is already pending.
- Turns played before a verified roll exists.

The previous `RecordRoll` action was removed completely. A player can no longer
inject privately selected dice through the version-2 action protocol.

### Canonical Fixtures

Protocol-version-1 fixtures were retained for historical compatibility testing.

Protocol-version-2 fixtures were added for replay state containing the pending
fair-dice round:

- `canonical-replay-state-v2.cbor`
- `canonical-replay-state-v2.blake3`

The version-2 canonical CBOR fixture is 851 bytes. The state hash is 32 bytes.

### Tested

Final local test results:

- `backgammon-core`: 44 tests passed.
- `backgammon-protocol`: 59 tests passed.
- `backgammon-contract`: 27 tests passed.
- Total: 130 tests passed.
- Documentation tests: passed.
- Workspace compilation check: passed.
- Release WebAssembly contract build: passed.
- Freenet package verification: passed.

The tests cover rules enforcement, deterministic replay, malformed records,
forged hashes, synchronization deltas, alternate delivery groupings, fair-dice
commitments and reveals, rejection sampling, state reconstruction, and
complete-turn execution.

### Contract Build

Verified contract build:

- Contract code hash: `7GgvRv6g1cFwSVSrbSMUaBNexZoCERkrZR2ojaQwvFLs`
- Raw WebAssembly size: 327231 bytes.
- Freenet package size: 327271 bytes.
- Package wrapper difference: 40 bytes.

The build script independently extracts the packaged WebAssembly payload and
verifies that it exactly matches the raw release artifact.

### Production Runtime Proof

The earlier generic ledger contract was executed successfully through the
Freenet Core 0.2.116 production Wasmtime runtime rather than only through native
Rust tests.

That proof established that the contract packaging and exported runtime path
work in the production execution environment. The current backgammon-specific
version-2 contract must still receive its own runtime-harness and two-client
integration tests.

### What Works

The project now has a complete transport-independent foundation for playing and
verifying a standard backgammon game.

A game can be represented as an authenticated-style ordered action history,
reconstructed deterministically, validated against the rules engine,
synchronized through contract deltas, and advanced using reproducible
commit-and-reveal dice.

### What Remains Uncertain

- The graphical browser client has not yet been implemented.
- Local two-player interaction has not yet been tested through a user interface.
- Player authentication and signing are not yet integrated.
- The policy for a player who commits but refuses to reveal still needs to be defined.
- The current backgammon contract has not yet been exercised through two independent Freenet clients.
- Real Freenet message delay, reordering, disconnection, and reconnection behavior remain untested.
- Player discovery, availability announcements, challenges, and challenge expiration remain unimplemented.
- Publication and protocol-upgrade behavior remain untested.

### Freenet Limitations Discovered

Freenet contract updates must be treated as asynchronous and potentially
grouped, delayed, duplicated, or delivered in different valid partitions.

The contract therefore cannot depend on ordinary request-response timing,
WebSockets, a central mutable database, or delivery boundaries matching
game-action boundaries.

Tests now confirm that complete histories converge when the same valid actions
arrive separately, together, or in different groupings. Actual network latency
and cross-node behavior still require measurement.

### Next Session

Begin the `local-client-0.1` milestone:

- Create the graphical backgammon board.
- Display all 24 points, checker stacks, bar, bear-off areas, dice, players, score, turn status, connection status, and move history.
- Connect checker selection and legal-destination highlighting directly to `backgammon-core`.
- Support a complete local two-player game before introducing Freenet communication.
