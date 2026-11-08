export const ROUTES = [
  { id: "sessions", label: "Sessions" },
  { id: "record", label: "Record" },
  { id: "queue", label: "Queue" },
  { id: "review", label: "Review" },
  { id: "system", label: "System" },
  { id: "settings", label: "Settings" },
] as const;

export type RouteId = (typeof ROUTES)[number]["id"];

/** Where the app opens: System on first launch (no capability profile yet), else Sessions. */
export function initialRoute(hasProfile: boolean): RouteId {
  return hasProfile ? "sessions" : "system";
}
