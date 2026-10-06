# River volunteer game milestone — October 6, 2026

Dada completed a Freenet game with River volunteer James. Dada played
White; James played Black. Dada's screenshot showed all 15 White checkers
borne off and a single-game win for one point. James reported seeing
Dada as the winner.

Dada's client reported 307 verified network actions, 206111 retrieved
bytes, and abbreviated ledger hash 9ff81feb…d0590d94. Independent final
snapshot comparison and offline replay were not performed.

Game: 0bbb276d424487ba114f1824d98d7cc7e0403649c8dc68673c07ed562710afb6
Contract: 6VvB72SZS3VxGy3PMb4F59fDZEsDzEKtztEVByj17Uyo
Fresh lobby: 5zrqJg6wYXvgD7ifJW3B5JDkQ9MSb4zPi3cb38PamqQv
Website: 38czsTGsDqUtMyd1TzW4BwX3t1rGd12QuNen7mbn2jcb
Published website version: 1791314950
Client asset identifier: c874a6bdd252a35c

This commit preserves lobby/rematch transitions, invitation controls,
history presentation, fresh-lobby scoping, serving-node WebSocket
selection, and genesis publication retry after current-connection
durable delegate registration.

Apply-script validation passed client tests and the release wasm32
client check. Fresh-lobby validation also passed protocol tests.
All four pinned delegate WASM files retained their exact bytes.

Open issues:
- Presence revision reservation hung after game activation; refreshing
  restored presence. Source inspection found missing presence reply
  handling in the replacement connection. No fix is included yet.
- Player names remained generic on the board.
- Completion presentation lost selected-game state, showed zero scores,
  and retained an inconsistent archive description.
- Winner-only stars and streamers are requested but not implemented.
- Ivor's separate accepted game remained awaiting genesis confirmation;
  recovery after the registration fix was not confirmed.
- Concurrent game switching and background operation need explicit tests.
- The original Freenet propagation failure remains unresolved.
