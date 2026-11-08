// Typed wrappers around the app's Tauri commands (src-tauri/src). Every call can fail; callers
// surface the error with a Toast.
import { invoke } from "@tauri-apps/api/core";

/** True when the first-launch system check has saved capability.json on this machine. */
export function hasCapabilityProfile(): Promise<boolean> {
  return invoke<boolean>("has_capability_profile");
}
