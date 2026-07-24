import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

type JsonObject = Record<string, unknown>;
type Predicate = (message: JsonObject) => boolean;

const protocolMajor = 1;
const repositoryRoot = resolve(import.meta.dir, "..");
const defaultBinary = join(
  repositoryRoot,
  "target",
  "debug",
  process.platform === "win32" ? "rust-doctor.exe" : "rust-doctor",
);
const binary = resolve(process.env.RUST_DOCTOR_LSP_BINARY ?? defaultBinary);

function object(value: unknown, label: string): JsonObject {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`${label} is not an object`);
  }
  return value as JsonObject;
}

function diagnostics(message: JsonObject): JsonObject[] {
  const params = object(message.params, "publishDiagnostics params");
  const values = params.diagnostics;
  if (!Array.isArray(values)) throw new Error("publishDiagnostics omitted diagnostics");
  return values.map((value, index) => object(value, `diagnostic ${index}`));
}

class LspClient {
  readonly child: ChildProcessWithoutNullStreams;
  readonly stderr: string[] = [];
  #buffer = Buffer.alloc(0);
  #messages: JsonObject[] = [];
  #waiters: Array<{
    predicate: Predicate;
    resolve: (message: JsonObject) => void;
    reject: (error: Error) => void;
    timer: ReturnType<typeof setTimeout>;
  }> = [];

  constructor(cwd: string) {
    this.child = spawn(binary, ["--lsp"], {
      cwd,
      env: { ...process.env, RUST_LOG: "rust_doctor=info" },
      stdio: ["pipe", "pipe", "pipe"],
      windowsHide: true,
    });
    this.child.stdout.on("data", (chunk: Buffer) => this.#receive(chunk));
    this.child.stderr.on("data", (chunk: Buffer) => this.stderr.push(chunk.toString()));
    this.child.on("error", (error) => this.#rejectAll(error));
    this.child.on("exit", (code, signal) => {
      if (this.#waiters.length > 0) {
        this.#rejectAll(
          new Error(
            `language server exited before the expected message: code=${String(code)} signal=${String(signal)} stderr=${this.stderr.join("")}`,
          ),
        );
      }
    });
  }

  send(message: JsonObject): void {
    const body = Buffer.from(JSON.stringify(message));
    this.child.stdin.write(`Content-Length: ${body.length}\r\n\r\n`);
    this.child.stdin.write(body);
  }

  waitFor(predicate: Predicate, label: string, timeoutMs = 10_000): Promise<JsonObject> {
    const existing = this.#messages.find(predicate);
    if (existing) return Promise.resolve(existing);
    return new Promise((resolveMessage, rejectMessage) => {
      const timer = setTimeout(() => {
        this.#waiters = this.#waiters.filter((waiter) => waiter.timer !== timer);
        rejectMessage(
          new Error(
            `${label} timed out after ${timeoutMs} ms; stderr=${this.stderr.join("")}`,
          ),
        );
      }, timeoutMs);
      this.#waiters.push({
        predicate,
        resolve: resolveMessage,
        reject: rejectMessage,
        timer,
      });
    });
  }

  stop(): void {
    this.child.kill("SIGKILL");
  }

  #receive(chunk: Buffer): void {
    this.#buffer = Buffer.concat([this.#buffer, chunk]);
    while (true) {
      const headerEnd = this.#buffer.indexOf("\r\n\r\n");
      if (headerEnd < 0) return;
      const header = this.#buffer.subarray(0, headerEnd).toString("ascii");
      const lengthText = header
        .split("\r\n")
        .find((line) => line.toLowerCase().startsWith("content-length:"))
        ?.split(":")[1]
        ?.trim();
      const length = Number(lengthText);
      if (!Number.isSafeInteger(length) || length < 0) {
        this.#rejectAll(new Error(`invalid LSP Content-Length header: ${header}`));
        return;
      }
      const bodyStart = headerEnd + 4;
      const bodyEnd = bodyStart + length;
      if (this.#buffer.length < bodyEnd) return;
      const body = this.#buffer.subarray(bodyStart, bodyEnd).toString("utf8");
      this.#buffer = this.#buffer.subarray(bodyEnd);
      let message: JsonObject;
      try {
        message = object(JSON.parse(body) as unknown, "LSP message");
      } catch (error: unknown) {
        this.#rejectAll(
          error instanceof Error ? error : new Error(`invalid LSP message: ${String(error)}`),
        );
        return;
      }
      this.#messages.push(message);
      for (const waiter of [...this.#waiters]) {
        if (!waiter.predicate(message)) continue;
        clearTimeout(waiter.timer);
        this.#waiters = this.#waiters.filter((candidate) => candidate !== waiter);
        waiter.resolve(message);
      }
    }
  }

  #rejectAll(error: Error): void {
    for (const waiter of this.#waiters) {
      clearTimeout(waiter.timer);
      waiter.reject(error);
    }
    this.#waiters = [];
  }
}

function initializeMessage(id: number, rootUri: string, requestedProtocol: number): JsonObject {
  return {
    jsonrpc: "2.0",
    id,
    method: "initialize",
    params: {
      processId: process.pid,
      rootUri,
      capabilities: { general: { positionEncodings: ["utf-16"] } },
      initializationOptions: {
        protocolMajor: requestedProtocol,
        debounceMs: 50,
        onSaveProjectChecks: true,
        projectBudgetMs: 1_000,
      },
    },
  };
}

async function compatibleLifecycle(fixture: string): Promise<void> {
  const client = new LspClient(fixture);
  const rootUri = pathToFileURL(fixture).href;
  const documentPath = join(fixture, "src", "lib.rs");
  const documentUri = pathToFileURL(documentPath).href;
  const validSource = "pub fn value(input: Option<u8>) -> u8 { input.unwrap() }\n";
  try {
    client.send(initializeMessage(1, rootUri, protocolMajor));
    const initialized = await client.waitFor((message) => message.id === 1, "initialize");
    const result = object(initialized.result, "initialize result");
    const capabilities = object(result.capabilities, "server capabilities");
    const experimental = object(capabilities.experimental, "experimental capabilities");
    if (experimental.rustDoctorProtocolVersion !== protocolMajor) {
      throw new Error("server advertised an incompatible Rust Doctor protocol");
    }
    client.send({ jsonrpc: "2.0", method: "initialized", params: {} });
    client.send({
      jsonrpc: "2.0",
      method: "textDocument/didOpen",
      params: {
        textDocument: {
          uri: documentUri,
          languageId: "rust",
          version: 1,
          text: validSource,
        },
      },
    });
    const firstPublish = await client.waitFor(
      (message) =>
        message.method === "textDocument/publishDiagnostics" &&
        object(message.params, "diagnostic params").version === 1,
      "initial diagnostics",
    );
    const firstDiagnostics = diagnostics(firstPublish);
    if (firstDiagnostics.length === 0) throw new Error("real server emitted no diagnostics");
    const firstData = object(firstDiagnostics[0]?.data, "diagnostic data");
    if (typeof firstData.canonical_id !== "string" || firstData.degraded !== false) {
      throw new Error("diagnostic omitted canonical identity or reported degraded initial state");
    }

    client.send({
      jsonrpc: "2.0",
      method: "textDocument/didSave",
      params: { textDocument: { uri: documentUri } },
    });
    await client.waitFor(
      (message) =>
        message.method === "window/logMessage" &&
        String(object(message.params, "log params").message).includes(
          "did not complete within its budget",
        ),
      "hard on-save budget",
      3_000,
    );

    client.send({
      jsonrpc: "2.0",
      method: "textDocument/didChange",
      params: {
        textDocument: { uri: documentUri, version: 2 },
        contentChanges: [{ text: "pub fn value(" }],
      },
    });
    const degradedPublish = await client.waitFor(
      (message) =>
        message.method === "textDocument/publishDiagnostics" &&
        object(message.params, "diagnostic params").version === 2,
      "degraded diagnostics",
    );
    const degradedDiagnostics = diagnostics(degradedPublish);
    if (degradedDiagnostics.length !== firstDiagnostics.length) {
      throw new Error("invalid syntax cleared last-known-good diagnostics");
    }
    if (
      degradedDiagnostics.some(
        (diagnostic) => object(diagnostic.data, "degraded diagnostic data").degraded !== true,
      )
    ) {
      throw new Error("degraded refresh was not exposed in diagnostic data");
    }

    client.send({
      jsonrpc: "2.0",
      method: "textDocument/didClose",
      params: { textDocument: { uri: documentUri } },
    });
    const closed = await client.waitFor(
      (message) =>
        message.method === "textDocument/publishDiagnostics" &&
        object(message.params, "close diagnostics params").version === undefined &&
        diagnostics(message).length === 0,
      "close diagnostics",
    );
    if (diagnostics(closed).length !== 0) throw new Error("close did not clear diagnostics");

    client.send({ jsonrpc: "2.0", id: 2, method: "shutdown", params: null });
    const shutdown = await client.waitFor((message) => message.id === 2, "shutdown");
    if (!Object.hasOwn(shutdown, "result")) throw new Error("shutdown did not succeed");
    client.send({ jsonrpc: "2.0", method: "exit", params: null });
  } finally {
    client.stop();
  }
}

async function incompatibleProtocolIsRejected(fixture: string): Promise<void> {
  const client = new LspClient(fixture);
  try {
    client.send(initializeMessage(9, pathToFileURL(fixture).href, protocolMajor + 1));
    const response = await client.waitFor((message) => message.id === 9, "protocol rejection");
    const error = object(response.error, "protocol rejection error");
    if (error.code !== -32602 || !String(error.message).includes("protocol major")) {
      throw new Error(`incompatible protocol did not fail with invalid params: ${JSON.stringify(response)}`);
    }
  } finally {
    client.stop();
  }
}

if (!existsSync(binary)) {
  throw new Error(
    `Rust Doctor LSP binary does not exist at ${binary}; build it with cargo build --features lsp --bin rust-doctor`,
  );
}

const fixture = mkdtempSync(join(tmpdir(), "rust-doctor-editor-e2e-"));
try {
  mkdirSync(join(fixture, "src"));
  writeFileSync(
    join(fixture, "Cargo.toml"),
    '[package]\nname = "editor-e2e"\nversion = "0.1.0"\nedition = "2024"\nbuild = "build.rs"\n',
  );
  writeFileSync(
    join(fixture, "src", "lib.rs"),
    "pub fn value(input: Option<u8>) -> u8 { input.unwrap() }\n",
  );
  writeFileSync(
    join(fixture, "build.rs"),
    "fn main() { std::thread::sleep(std::time::Duration::from_secs(30)); }\n",
  );
  if (existsSync(join(fixture, "rust-doctor.toml"))) {
    throw new Error("fixture unexpectedly contains rust-doctor.toml");
  }
  await compatibleLifecycle(fixture);
  await incompatibleProtocolIsRejected(fixture);
  console.log(`LSP E2E passed with ${basename(binary)} and no project config`);
} finally {
  rmSync(fixture, { recursive: true, force: true });
}
