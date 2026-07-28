import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;

class NivrenDebugAdapterFactory implements vscode.DebugAdapterDescriptorFactory {
  public constructor(private readonly executable: string) {}

  public createDebugAdapterDescriptor(): vscode.DebugAdapterDescriptor {
    return new vscode.DebugAdapterExecutable(this.executable, ["dap"]);
  }
}

class NivrenDebugConfigurationProvider implements vscode.DebugConfigurationProvider {
  public resolveDebugConfiguration(
    _folder: vscode.WorkspaceFolder | undefined,
    configuration: vscode.DebugConfiguration,
  ): vscode.ProviderResult<vscode.DebugConfiguration> {
    if (!configuration.type && !configuration.request && !configuration.name) {
      configuration.type = "nivren";
      configuration.request = "launch";
      configuration.name = "Launch Nivren";
    }
    if (!configuration.program) {
      const editor = vscode.window.activeTextEditor;
      if (editor?.document.languageId !== "nivren") {
        return undefined;
      }
      configuration.program = editor.document.uri.fsPath;
    }
    return configuration;
  }
}

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
  context.subscriptions.push(
    vscode.debug.registerDebugAdapterDescriptorFactory(
      "nivren",
      new NivrenDebugAdapterFactory(executable),
    ),
    vscode.debug.registerDebugConfigurationProvider(
      "nivren",
      new NivrenDebugConfigurationProvider(),
    ),
  );
  await client.start();
}

export async function deactivate(): Promise<void> {
  if (client) {
    await client.stop();
    client = undefined;
  }
}
