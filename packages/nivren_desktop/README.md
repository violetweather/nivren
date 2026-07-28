# nivren_desktop

Typed contracts for the experimental Nivren desktop host. It validates system-webview window configuration, caps bridge messages at 1 MiB, serializes bridge requests through a derived JSON schema, and validates signed HTTPS update metadata for stable, beta, and nightly channels.

The native host owns the operating-system webview, code signing, package signing, and update installation. It exposes only the checked ABI v3 message boundary; web content does not receive raw native handles. Desktop support remains experimental until clean Windows, macOS, and Linux packaging/signing/update evidence passes Product Proof.
