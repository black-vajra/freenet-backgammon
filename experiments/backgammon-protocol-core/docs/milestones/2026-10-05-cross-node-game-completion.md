# Two-node game completion and rematch test — October 5, 2026

## Observed milestone

Pots (Dada / Black, Kubuntu) and EliteBook/vulfen
(Mama / White, SableLinux) recovered the existing network game
and played it through completion. The client reported:
Black wins, gammon, two points.

Game:
2fbcd1d1b59ff64a9046a99755b0a91054f8114fc5a558de5b75d5f3a4a29d2c

Contract:
Do2kfigXWpiN6hYQ4D4sbajEe8ztD5e3jF2vBzBSxnxH

Both clients reported 301 verified network actions. Independent
GETs through the two local nodes returned identical final bytes:

- Size: 201,519 bytes
- SHA-256:
  6c8791327cef6ec3abc7383bf44627f58c8f529c4dff46e8a540688a6fea7469

This establishes byte-for-byte agreement on the captured completed
ledger. It does not substitute for independent deterministic replay.

## Recovery and delivery observations

The session began with Pots returning the previously saved
133-action state and Mama returning the larger pending-White-roll
state. Before Pots' diagnostic restart, its GET had caught up to
Mama: both returned 90,902 bytes with SHA-256
b6f5800531a02a1e3bf60a8e747b7335abdd710b3491d7384c7983e470ff36ee.

The mechanism and exact time of that convergence were not established.

Both nodes retained those bytes across their diagnostic restarts.
After reopening the existing clients, White's pending roll recovered
as 5 and 1. Subsequent dice exchanges and moves propagated during
live play, with delivery observed within seconds. A fully blocked
White turn was successfully passed. No manual action transfers were
reported during this resumed session.

These observations demonstrate successful live delivery during this
session; they do not prove that the earlier propagation defect is fixed.

## Diagnostic executable

Both running processes were verified against the same executable:

- Version: Freenet 0.2.135
- Source commit: ea1ff5f169bc0a2279dd201f4463a76bfad79cb4
- Existing canonical-key/placeholder-code-hash UPDATE patch retained
- Tracing feature changed from release_max_level_info to
  release_max_level_trace in a separate diagnostic checkout
- Binary SHA-256:
  eda99558bee1ac95a1e6dd418ab9594f533cbcabbe2a837af3723c19402f03b8

Runtime DEBUG targets were operations::update,
operations::subscribe, and ring::interest. General client/delegate
DEBUG logging was not enabled. Auto-update remained disabled.

## Saved evidence on the test machines

Pots:
- /home/pepper/backgammon-diagnostic-pots-DNndT3Zn
- /home/pepper/backgammon-completed-OoOSg0ky

Mama:
- /home/sable/backgammon-diagnostic-mama-oQh4wgSb
- /home/sable/backgammon-completed-hlC45hAl

The diagnostic directories contain before/after captures and saved
logs. The completed directories contain completed.cbor and
game-session.log.

## Rematch and remaining work

Both players offered rematches with switched colors. Mama accepted
Dada's offer. Both clients listed the same new accepted game,
ae71b751…38e155d6, while retaining the completed game as their
selected active scope.

Source inspection confirmed that Play Again opens challenge choices;
acceptance does not itself select and activate the accepted game.
Both players manually selected and activated the new game. A fresh
board then appeared, with Mama as Black and Dada as White.

Remaining work:
- Make rematch selection and activation clear in the normal game flow.
- Add Quit / Return to lobby to the completion dialog.
- Investigate presence stuck at "Reserving next presence revision".
- Diagnose the earlier UPDATE delivery/reconciliation stall.
- Independently replay the completed ledger and broaden live testing.

The tested client source remains substantially uncommitted. This
milestone commit records observations and does not commit that source.
