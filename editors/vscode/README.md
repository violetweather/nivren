# Nivren for VS Code

This extension provides Edition 4 syntax highlighting, intent-first diagnostics, completion, canonical formatting, and debugger launch support for `.niv` files.

Install the Nivren toolchain first and ensure `niv` is on `PATH`. If it is elsewhere, set **Nivren: Server Path** to the executable's absolute path. The extension starts `niv lsp` automatically when a Nivren file opens.

Completion includes Edition 4 declarations, labeled calls, capabilities, resources, derives, and shape-derived JSON entry points such as `std.json.decode`, `std.json.read_next`, and `std.json.read_next_as`.

The Run and Debug view registers the Nivren Debug Adapter. Open a `.niv` file and choose **Launch Nivren**, or create a launch configuration with `"type": "nivren"`, `"request": "launch"`, and a source-file or project `"program"`. The extension starts the adapter as `niv dap`, using the same configured executable as the language server.

## Development

```text
npm ci
npm run check
npm run compile
```
