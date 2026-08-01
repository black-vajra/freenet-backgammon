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
