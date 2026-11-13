/**
 * Why live captions are not offered yet. A disabled live option always shows its reason
 * (design-consistency rule); System and Record use the same wording.
 */
export function liveUnavailableReason(hasProfile: boolean): string {
  return hasProfile
    ? "Live captions are chosen in the Start Session dialog, and only where the system check cleared this computer."
    : "Not offered until the system check has run.";
}
