export const RUST_DOCTOR_PROTOCOL_MAJOR = 1;

export function resolveBinaryPath(configured: string, platform: NodeJS.Platform): string {
  const value = configured.trim();
  if (value && value !== "rust-doctor") return value;
  return platform === "win32" ? "rust-doctor.exe" : "rust-doctor";
}

export function isCompatibleVersion(output: string): boolean {
  const match = output.trim().match(/^rust-doctor (\d+)\.(\d+)\.(\d+)/);
  if (!match) return false;
  const major = Number(match[1]);
  const minor = Number(match[2]);
  return major > 0 || minor >= 2;
}

export function isCompatibleProtocol(output: string): boolean {
  const match = output.match(/^lsp-protocol (\d+)$/m);
  return match !== null && Number(match[1]) === RUST_DOCTOR_PROTOCOL_MAJOR;
}

export interface EditorInitializationSettings {
  debounceMs: number;
  onSaveProjectChecks: boolean;
  projectBudgetMs: number;
  configurationPath: string;
}

export function initializationOptions(
  configuration: EditorInitializationSettings,
): Record<string, unknown> {
  const options: Record<string, unknown> = {
    protocolMajor: RUST_DOCTOR_PROTOCOL_MAJOR,
    debounceMs: configuration.debounceMs,
    onSaveProjectChecks: configuration.onSaveProjectChecks,
    projectBudgetMs: configuration.projectBudgetMs,
  };
  const configurationPath = configuration.configurationPath.trim();
  if (configurationPath) options.configurationPath = configurationPath;
  return options;
}
