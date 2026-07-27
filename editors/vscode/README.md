# Nivren for VS Code

This extension provides Edition 3 syntax highlighting, intent-first diagnostics, completion (including `Random`, `SecretKey`, password hashing, and authenticated encryption), and document formatting for `.niv` files.

Install the Nivren toolchain first and ensure `niv` is on `PATH`. If it is elsewhere, set **Nivren: Server Path** to the executable's absolute path. The extension starts `niv lsp` automatically when a Nivren file opens.

Completion includes Edition 3's shape-derived JSON entry points: `std.json.decode`, `std.json.read_next`, and `std.json.read_next_as`.

## Development

```text
npm ci
npm run check
npm run compile
```
