// Compatibility-only migration for benign browser preferences created by AMWAPOS.
// Security-sensitive state is intentionally excluded: backend sessions die with
// the old process, and setup drafts may contain an 8-digit hub pairing code.

export function readMigratedLocalStorage(currentKey: string, legacyKey: string): string | null {
  try {
    const current = localStorage.getItem(currentKey);
    if (current !== null) return current;
    const legacy = localStorage.getItem(legacyKey);
    if (legacy === null) return null;
    localStorage.setItem(currentKey, legacy);
    // Retain the legacy value for rollback; current TILLBH state takes precedence.
    return legacy;
  } catch {
    return null;
  }
}

export function migrateLegacyBrowserStorage(): void {
  readMigratedLocalStorage("tillbh.lang", "amwapos.lang");
  readMigratedLocalStorage("tillbh.ai.lang", "amwapos.ai.lang");
  // Deliberately do NOT migrate amwapos.session or amwapos.setup.draft.
}
