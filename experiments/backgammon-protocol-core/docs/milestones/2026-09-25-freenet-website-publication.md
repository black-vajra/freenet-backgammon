# Freenet Backgammon Website Publication — 2026-09-25

The Backgammon Yew/WASM client was built successfully as a production static site with Trunk 0.21.14 using:

    trunk build --release --dist /tmp/freenet-backgammon-website --public-url "./" index.html

The resulting bundle contained index.html plus hashed CSS, JavaScript, and WebAssembly assets. Asset paths were relative and the production build contained no Trunk live-reload WebSocket client.

A dedicated Freenet website identity was created:

    key name: freenet-backgammon
    contract ID: 9ZFiVUxL2yJuHnW2Dum3nhP7znRRGSHJjHFFqzmPbcWo

The signing key is stored outside the repository and was backed up separately.

The site was published successfully with:

    fdev network website publish --key freenet-backgammon --timeout 300 /tmp/freenet-backgammon-website

Published website version:

    1790367690

Local URL:

    http://127.0.0.1:7509/v1/contract/web/9ZFiVUxL2yJuHnW2Dum3nhP7znRRGSHJjHFFqzmPbcWo/

Verification confirmed that Freenet serves the site through its sandbox shell, injects its FreenetWebSocket bridge, and serves the CSS, JavaScript, and WebAssembly assets with correct MIME types.

Interactive browser validation is still pending. The next step is to open the published site on pots and verify UI rendering, Freenet WebSocket connectivity, lobby retrieval, and lobby subscription.

No client source changes were required for this publication milestone.
