import type { Capability } from "../ipc";

/**
 * Why live captions are not offered yet. A disabled live option always shows its reason
 * (design-consistency rule); System and Record use the same wording.
 */
export function liveUnavailableReason(hasProfile: boolean, cap?: Capability | null): string {
  // With a profile in hand, give the profile's own reason for the first language live is not offered for.
  if (cap) {
    const any = Object.values(cap.decision.languages).find((l) => l && !l.live.offered);
    const own = any?.live.hard_reasons[0] ?? any?.live.soft_reasons[0];
    if (own) return own;
  }
  return hasProfile
    ? "Live captions are chosen in the Start Session dialog, and only where the system check cleared this computer."
    : "Not offered until the system check has run.";
}
