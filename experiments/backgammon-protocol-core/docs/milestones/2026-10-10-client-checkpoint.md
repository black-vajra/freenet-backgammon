# Freenet Backgammon client checkpoint — 10 October 2026

This records the source reviewed in ChatGPT and build/publication evidence supplied from Pots. The live checkout is on Pots; ChatGPT has no direct access to either running node.

## Source and UI

Preserve the current `network-actions-0.1` client: User manual with show/hide; in-game lobby availability and invitation notices; challenger color selection captured from the actual opponent-row selector; fixed recipient colors with signed Accept/Decline; one full invitation per advertised opponent; private Hidden and Archive lists with Restore; and winner presentation based on the verified game snapshot, with reduced-motion streamers. Existing presence-renewal work is included because the edited main module depends on it.

Archive uses version 2 private list preferences. Canonical version 1 hidden preferences are retained on read and migrated on the next explicit preference write using the original bytes for compare-and-replace. Hidden and Archive are disjoint; archived games are omitted from ordinary/Hidden lists and nonactive automatic historical probes. Restore removes the personal list preference. No resignation, cancellation, shared-game deletion, or opponent list mutation is performed. Older clients cannot read version 2 preferences; do not remove local data or downgrade the preferences module to work around that.

## Publication configuration

The addresses below supersede the reversed retirement plan from 7 October.

| Purpose | Website key | Contract |
| --- | --- | --- |
| Working client | backgammon-pending-20261004-204856 | 38czsTGsDqUtMyd1TzW4BwX3t1rGd12QuNen7mbn2jcb |
| Retired presence preview | backgammon-presence-preview-20261007T175615Z | 2mjb6wMHsMuYLn11wWTrHWU6FK8jghLhEhwLKe8qKvEL |
| Retired original website | freenet-backgammon | 9ZFiVUxL2yJuHnW2Dum3nhP7znRRGSHJjHFFqzmPbcWo |
| Retired durable preview | freenet-backgammon-durable-preview | JD5zDvZKVtyC9GgjswEGbbhECroXDhUC6Fa9kPRTEng6 |
| Retired cleanup preview | backgammon-cleanup-20261004-200306 | Hpzer5QCkSSPPgjcJgynE7hzx17fyhqsgk8zbG561v3B |
| Retired recovery preview | freenet-backgammon-recovery-preview-20261004 | Hk2eb7RycLPakXnUJvESX7QwqUNXHBz9oX2E7RKM9Eeb |

Current lobby in the supplied source: `5zrqJg6wYXvgD7ifJW3B5JDkQ9MSb4zPi3cb38PamqQv`. This is distinct from the website. Website navigation and WebSocket connections use the visitor's serving node; there is no Pots/EliteBook application server requirement.

The working document title and H1 are **Freenet Backgammon**. The retired 2mjb document title/H1 are **Freenet Backgammon retired**, with a warning and same-node link to the working address. These public page names are crawler input, not an authenticated Atlas index edit. Exact Atlas labels and recrawl timing are not confirmed.

## Build and observations

- Latest successful working-site publication: version **1791658754**, reported by Pots. Build directory: `/home/pepper/backgammon-archive-build-hZzzIy23/site`.
- Release build completed; packaged manual SHA-256: `bedb1e563fd5fa1a6f25aaea0ef5d591b7605d250075117c4ab6b0bb0ecf9d8a`. An initial PUT reported a cancelled streaming relay; retry of the same build succeeded. The earlier error alone did not establish whether the update had reached any peer.
- The supplied build pipeline requires targeted preference/challenge/result/transition tests and a wasm32 check before release construction. Do not substitute this build evidence for browser or independent-node validation.
- Live observations: color choice worked in both challenger directions after loading the updated client; one invitation in the opponent row removed the duplicate panel.
- The 2mjb retirement notice was observed on both nodes. Four other notices published successfully, versions 1791658039/1791658040. Subsequent Pots GETs returned 1058, 1062, 1053, and 1058 bytes respectively, replacing roughly 427–462 KB states. The first HTTP verifier inspected the gateway shell (`Freenet`, two scripts), so its stale-page flags were invalid. The corrected verifier requests the inner app using `?__sandbox=1`; actual execution of that corrected copy was not confirmed.
- The user subsequently reported the retirement/title work completed. No additional publication version or independent evidence for that final title update was supplied.

Pinned delegates unchanged (SHA-256):

| Delegate | SHA-256 |
| --- | --- |
| Identity/local state | d16d376df4f64d8b127b0bdab68eecb24b3d0a1c811202ead560312d17cdf87c |
| Profile | d292f0ba6b1259fad0557a5cfbc9f075eff2570e31a0c90a9dedd57b5a3dc1a8 |
| Presence revision | 65e432c9b2f6bdc9f10c98c6ed4ae73d4db9eed523d20aedd6351ccc7ea16569 |
| Durable state | e7a0446efab84cb6fa1491484f0d176e9d54fcdb677a90a9747608e508d2fe71 |

## Open items

1. Observe Archive → reload → Restore on each identity; verify cleanup is personal to that node and the same game remains recoverable. No direct archive browser test evidence is recorded here.
2. A forced pass displayed **Action storage requires recovery: An action is already pending; reconnect to recover it** while the board showed Black to roll. The game continued without interruption. Source confirms the message is triggered by an occupied durable pending slot; the exact action/race was not captured. It is a remaining UI/recovery defect: clear resolved warnings only after verified recovery, retaining actionable errors for genuinely unresolved storage. No fix is claimed.
3. Verify the four additional retired pages on an independent node and Atlas recrawling. Never initialize extra website keys for routine updates.
4. Validate two independent players accessing the latest website/lobby and creating/playing a game while both Pots and EliteBook are stopped. Freenet's replication design does not prove current reachable copies or durability when these particular nodes go offline.
5. Continue the existing lifecycle, delayed dice, replay and multi-game evidence plan. This checkpoint does not claim the underlying propagation problem is fixed.

The operator intends to pause their two nodes after finishing the current game. No shutdown, disablement, node-storage reset, key deletion or live service-config change is performed by this checkpoint/commit procedure. Preserve node identity and recovery data for a later return.
