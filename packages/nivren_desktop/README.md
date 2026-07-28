# nivren_desktop

Typed contracts and a capability-scoped adapter for the experimental Nivren desktop host. It validates system-webview windows, caps bridge messages at 1 MiB, serializes requests through derived JSON, and validates signed HTTPS update metadata. `open_host`, `send_bridge`, and `stage_update` use one opaque `Native within "desktop"` handle with deterministic `using` cleanup.

The native host owns the operating-system webview, origin/CSP/command enforcement, code signing, package signing, and update installation. It exposes only the checked ABI v3 message boundary; web content does not receive raw native handles. VM/native-control Product Proof verifies identical bridge operations and exactly-once cleanup. Desktop support remains experimental until clean Windows, macOS, and Linux packaging/signing/update evidence passes.
