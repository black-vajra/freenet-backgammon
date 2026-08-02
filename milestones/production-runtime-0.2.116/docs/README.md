# Freenet Production-Runtime Milestone

Date: 2026-08-01
Freenet Core: 0.2.116
Contract: ledger_prototype
Runtime: Freenet Core production Wasmtime backend

## Result

The ledger prototype successfully executes through Freenet Core 0.2.116's
production WASM runtime.

The verified integration test is:

    wasm_runtime::tests::ledger_prototype::ledger_package_runs_through_production_wasm_runtime

Final result:

    test result: ok. 1 passed; 0 failed; 0 ignored

## What was verified

- The Rust contract compiles to wasm32-unknown-unknown.
- fdev produces a Freenet package with a 40-byte header.
- The package payload exactly matches the compiled raw WASM.
- Freenet Core loads the contract through its production Wasmtime backend.
- Initial state validation succeeds.
- A valid ledger update is accepted.
- The returned state decodes correctly.
- Duplicate and invalid actions are rejected by the test harness.
- Returned state bytes exactly equal independently encoded expected bytes.
- The focused runtime integration test passes from a permanent location
  outside /tmp.

## Reproduction command

Run from a Freenet Core 0.2.116 checkout containing the preserved harness:

    LEDGER_WASM_PACKAGE=/path/to/ledger_prototype.wasm \
    CARGO_TARGET_DIR=/var/tmp/freenet-core-02116-target \
    cargo test \
      -p freenet \
      --lib \
      --features wasmtime-backend \
      -j 1 \
      'wasm_runtime::tests::ledger_prototype::ledger_package_runs_through_production_wasm_runtime' \
      -- \
      --exact \
      --nocapture

Despite the historical environment-variable name, the current harness receives
the raw WASM module through LEDGER_WASM_PACKAGE. The corresponding Freenet
package is preserved separately and verified as a 40-byte header followed by
the exact raw module.

## Preserved evidence

- artifacts/ledger_prototype.wasm
- artifacts/ledger_prototype.freenet-package
- contract-source/
- harness/ledger_prototype.rs
- logs/production-runtime-clean-final-test.txt
- SHA256SUMS

## Scope

This milestone establishes production-runtime compatibility for the minimal
append-only ledger prototype. It does not yet establish complete backgammon
rules, authentication, fair dice, lobby behavior, or live two-client Freenet
synchronization.

## Next milestone

Extend the ledger protocol with a small versioned game-action envelope and
production-runtime tests for:

- sequential action numbers;
- stale and duplicate rejection;
- deterministic reconstruction;
- resulting-state hashes;
- player authentication fields; and
- malformed or conflicting updates.
