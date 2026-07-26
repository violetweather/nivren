# Nivren for VS Code

This extension provides syntax highlighting, diagnostics, completion, and document formatting for `.niv` files.

Install the Nivren toolchain first and ensure `niv` is on `PATH`. If it is elsewhere, set **Nivren: Server Path** to the executable's absolute path. The extension starts `niv lsp` automatically when a Nivren file opens.

## Development

```text
npm ci
npm run check
npm run compile
```
