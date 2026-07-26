import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  const executable = vscode.workspace
    .getConfiguration("nivren")
    .get<string>("server.path", "niv");
  const serverOptions: ServerOptions = {
    command: executable,
    args: ["lsp"],
  };
  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: "nivren" }],
    synchronize: {
      fileEvents: vscode.workspace.createFileSystemWatcher("**/*.niv"),
    },
  };
  client = new LanguageClient(
    "nivrenLanguageServer",
    "Nivren Language Server",
    serverOptions,
    clientOptions,
  );
  context.subscriptions.push(client);
  await client.start();
}

export async function deactivate(): Promise<void> {
  if (client) {
    await client.stop();
    client = undefined;
  }
}
