// Tiny localStorage wrapper for non-sensitive server settings. The admin
// token is stored through the Tauri command layer in the system keychain.

import { loadAdminToken as loadKeychainAdminToken, saveAdminToken as saveKeychainAdminToken } from "./commands";

const KEY_BASE_URL = "inkwash.server.baseUrl";
const LEGACY_KEY_ADMIN_TOKEN = "inkwash.server.adminToken";
const KEY_SELECTED_DEVICE = "inkwash.server.selectedDevice";

export function loadServerBaseUrl(): string {
  return localStorage.getItem(KEY_BASE_URL) ?? "";
}

export function saveServerBaseUrl(value: string): void {
  if (value) localStorage.setItem(KEY_BASE_URL, value);
  else localStorage.removeItem(KEY_BASE_URL);
}

export async function loadAdminToken(): Promise<string> {
  const stored = await loadKeychainAdminToken();
  if (!stored.ok) throw new Error(stored.error.message);
  if (stored.value) return stored.value;

  // Migrate one existing plaintext token, then remove the old entry only
  // after the keychain write succeeds.
  const legacy = localStorage.getItem(LEGACY_KEY_ADMIN_TOKEN);
  if (!legacy) return "";
  const migrated = await saveKeychainAdminToken(legacy);
  if (!migrated.ok) throw new Error(migrated.error.message);
  localStorage.removeItem(LEGACY_KEY_ADMIN_TOKEN);
  return legacy;
}

export async function saveAdminToken(value: string): Promise<void> {
  const result = await saveKeychainAdminToken(value);
  if (!result.ok) throw new Error(result.error.message);
  localStorage.removeItem(LEGACY_KEY_ADMIN_TOKEN);
}

export function loadSelectedDeviceId(): string | null {
  const raw = localStorage.getItem(KEY_SELECTED_DEVICE);
  if (!raw) return null;
  // Device ids are now UUID strings; a leftover numeric id from before
  // the UUID migration can never match a real device, so drop it.
  return raw.trim() ? raw : null;
}

export function saveSelectedDeviceId(id: string | null): void {
  if (id == null) localStorage.removeItem(KEY_SELECTED_DEVICE);
  else localStorage.setItem(KEY_SELECTED_DEVICE, id);
}
