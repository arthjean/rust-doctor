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
