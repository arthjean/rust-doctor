import { spawnSync } from "node:child_process";
import {
  commands,
  env,
  ExtensionContext,
  languages,
  Position,
  Range,
  Uri,
  window,
  workspace,
} from "vscode";
import {
  Executable,
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";
import {
  isCompatibleProtocol,
  isCompatibleVersion,
  initializationOptions,
  resolveBinaryPath,
  RUST_DOCTOR_PROTOCOL_MAJOR,
} from "./binary";

let client: LanguageClient | undefined;
let startupErrorShown = false;

interface RustDoctorSettings {
  enabled: boolean;
  binaryPath: string;
  debounceMs: number;
  onSaveProjectChecks: boolean;
  projectBudgetMs: number;
  configurationPath: string;
  trace: "off" | "messages" | "verbose";
}

function settings(): RustDoctorSettings {
  const configuration = workspace.getConfiguration("rustDoctor");
  return {
    enabled: configuration.get("enabled", true),
    binaryPath: configuration.get("binaryPath", "rust-doctor"),
    debounceMs: configuration.get("debounceMs", 300),
    onSaveProjectChecks: configuration.get("onSaveProjectChecks", false),
    projectBudgetMs: configuration.get("projectBudgetMs", 10_000),
    configurationPath: configuration.get("configurationPath", ""),
    trace: configuration.get("trace", "off"),
  };
}

function traceEnvironment(level: RustDoctorSettings["trace"]): NodeJS.ProcessEnv {
  const environment = { ...process.env };
  if (level === "messages") environment.RUST_LOG = "rust_doctor=debug";
  if (level === "verbose") environment.RUST_LOG = "rust_doctor=trace";
  return environment;
}

function verifyBinary(binaryPath: string): string | undefined {
  const result = spawnSync(binaryPath, ["version"], {
    encoding: "utf8",
    timeout: 5_000,
    windowsHide: true,
  });
  if (result.error) return `${binaryPath}: ${result.error.message}`;
  if (result.status !== 0) return `${binaryPath} exited with status ${result.status ?? "unknown"}`;
  if (!isCompatibleVersion(result.stdout)) {
    return `${binaryPath} is incompatible; Rust Doctor 0.2.0 or newer is required`;
  }
  if (!isCompatibleProtocol(result.stdout)) {
    return `${binaryPath} does not provide Rust Doctor LSP protocol ${RUST_DOCTOR_PROTOCOL_MAJOR}`;
  }
  return undefined;
}

async function startClient(): Promise<void> {
  const configuration = settings();
  if (!configuration.enabled) return;
  const binaryPath = resolveBinaryPath(configuration.binaryPath, process.platform);
  const binaryError = verifyBinary(binaryPath);
  if (binaryError) {
    if (!startupErrorShown) {
      startupErrorShown = true;
      await window.showErrorMessage(
        `Rust Doctor diagnostics disabled: ${binaryError}. Set rustDoctor.binaryPath to a binary built with --features lsp.`,
      );
    }
    return;
  }

  const executable: Executable = {
    command: binaryPath,
    args: ["--lsp"],
    transport: TransportKind.stdio,
    options: { env: traceEnvironment(configuration.trace) },
  };
  const serverOptions: ServerOptions = executable;
  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: "rust" }],
    diagnosticCollectionName: "rust-doctor",
    initializationOptions: initializationOptions(configuration),
    connectionOptions: { maxRestartCount: 0 },
  };
  client = new LanguageClient("rustDoctor", "Rust Doctor", serverOptions, clientOptions);
  try {
    await client.start();
  } catch (error: unknown) {
    if (!startupErrorShown) {
      startupErrorShown = true;
      const message = error instanceof Error ? error.message : String(error);
      await window.showErrorMessage(`Rust Doctor diagnostics disabled: ${message}`);
    }
    client = undefined;
  }
}

function binaryTerminal(name: string, args: string[]): void {
  const terminal = window.createTerminal({
    name,
    shellPath: settings().binaryPath,
    shellArgs: args,
  });
  terminal.show();
}

function activeRustDoctorDiagnostic(): { rule: string; line: number; column: number; file: string } | undefined {
  const editor = window.activeTextEditor;
  if (!editor) return undefined;
  const position = editor.selection.active;
  const diagnostic = languages
    .getDiagnostics(editor.document.uri)
    .find((candidate) => candidate.source === "rust-doctor" && candidate.range.contains(position));
  if (!diagnostic) return undefined;
  const rule =
    typeof diagnostic.code === "object" && diagnostic.code !== null
      ? diagnostic.code.value
      : diagnostic.code;
  if (!rule) return undefined;
  return {
    rule: String(rule),
    line: diagnostic.range.start.line + 1,
    column: diagnostic.range.start.character + 1,
    file: editor.document.uri.fsPath,
  };
}

export async function activate(context: ExtensionContext): Promise<void> {
  context.subscriptions.push(
    commands.registerCommand("rustDoctor.scanWorkspace", () => {
      const root = workspace.workspaceFolders?.[0]?.uri.fsPath ?? ".";
      binaryTerminal("Rust Doctor scan", [root]);
    }),
    commands.registerCommand("rustDoctor.explainDiagnostic", () => {
      const diagnostic = activeRustDoctorDiagnostic();
      if (!diagnostic) {
        void window.showInformationMessage("Place the cursor on a Rust Doctor diagnostic.");
        return;
      }
      binaryTerminal("Rust Doctor explanation", [
        "why",
        `${diagnostic.file}:${diagnostic.line}:${diagnostic.column}`,
        "--rule",
        diagnostic.rule,
      ]);
    }),
    commands.registerCommand("rustDoctor.openRuleDocumentation", async () => {
      const diagnostic = activeRustDoctorDiagnostic();
      if (!diagnostic) {
        await window.showInformationMessage("Place the cursor on a Rust Doctor diagnostic.");
        return;
      }
      await env.openExternal(Uri.parse(`https://rust-doctor.vercel.app/rules/${encodeURIComponent(diagnostic.rule)}`));
    }),
  );
  await startClient();
  if (client) context.subscriptions.push(client);
}

export async function deactivate(): Promise<void> {
  const activeClient = client;
  client = undefined;
  if (activeClient) await activeClient.stop();
}

export const protocolFixture = {
  position: new Position(0, 0),
  range: new Range(new Position(0, 0), new Position(0, 1)),
};
