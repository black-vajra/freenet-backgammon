# Cross-Node Freenet Lobby Contract Publication

Date: August 21, 2026

## Milestone

The standalone backgammon lobby contract was successfully built, published through one Freenet node, retrieved locally, and independently retrieved from a second Freenet node using a different Internet egress path.

The retrieved state on both nodes was byte-for-byte identical to the original published initial state.

This establishes that the lobby contract can be published and retrieved across independently connected Freenet nodes.

## Source State

Repository:

`~/Desktop/freenet-backgammon`

Workspace:

`experiments/backgammon-protocol-core`

Branch:

`network-actions-0.1`

Source checkpoint:

`50ceae2 Add Freenet lobby contract interface`

## Build Artifact

The lobby contract was built using:

`fdev network build`

Generated contract artifact:

`crates/backgammon-lobby-contract/build/freenet/backgammon_lobby_contract`

Contract ID:

`4dEpRHVuGu5P34i5pp8ify1GAueWN94wDfHNr9uZvZZB`

## Verification

Publishing node:
- Host: pots
- Freenet version: 0.2.125 patched release
- Publication succeeded through Freenet network

Independent retrieval node:
- Host: vulfen
- Freenet version: 0.2.120
- Separate Internet connection

Retrieved state:

17 bytes

SHA256:

`f82abcd1cf6e26ab1a5ccf23610fb36d554f5966b5dab500753ccdb9ed0eef35`

Result:

Byte-for-byte identical state retrieved from both nodes.

## Significance

This milestone demonstrates cross-node availability of the Freenet backgammon lobby contract.

The next phase is integration of real lobby presence announcements and challenge delivery over the replicated lobby state.
