/**
 * Why live captions are not offered yet. A disabled live option always shows its reason
 * (design-consistency rule); System and Record use the same wording.
 */
export function liveUnavailableReason(hasProfile: boolean): string {
  return hasProfile
    ? "Live captions are not built yet; the saved profile is not read in this version."
    : "Not offered until the system check has run.";
}
