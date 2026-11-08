import { useEffect, useState } from "react";
import { Toast } from "./ds";
import type { ToastTone } from "./ds";
import { hasCapabilityProfile } from "./ipc";
import { initialRoute, type RouteId } from "./routes";
import { QueueScreen } from "./screens/QueueScreen";
import { RecordScreen } from "./screens/RecordScreen";
import { ReviewScreen } from "./screens/ReviewScreen";
import { SessionsScreen } from "./screens/SessionsScreen";
import { SettingsScreen } from "./screens/SettingsScreen";
import { SystemScreen } from "./screens/SystemScreen";
import { StatusBar } from "./shell/StatusBar";
import { TopBar } from "./shell/TopBar";

const APP_VERSION = "0.1.0";

interface ToastState {
  tone: ToastTone;
  title: string;
  message?: string;
}

export function App() {
  const [route, setRoute] = useState<RouteId | null>(null);
  const [hasProfile, setHasProfile] = useState<boolean | null>(null);
  const [toast, setToast] = useState<ToastState | null>(null);

  useEffect(() => {
    let cancelled = false;
    hasCapabilityProfile()
      .then((found) => {
        if (cancelled) return;
        setHasProfile(found);
        setRoute(initialRoute(found));
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setHasProfile(false);
        setRoute(initialRoute(false));
        setToast({
          tone: "danger",
          title: "Could not look for the system check profile",
          message: err instanceof Error ? err.message : String(err),
        });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const profileText =
    hasProfile === null ? "Checking…" : hasProfile ? "System check saved" : "System check not run";

  return (
    <div
      style={{
        height: "100%",
        display: "grid",
        gridTemplateRows: "var(--app-bar-h) auto 1fr var(--app-status-h)",
        overflow: "hidden",
      }}
    >
      <TopBar active={route} onNavigate={setRoute} machineLabel={profileText} />
      <Screen route={route} hasProfile={!!hasProfile} />
      <StatusBar
        items={[
          <span key="v" style={{ fontFamily: "var(--font-mono)" }}>
            v{APP_VERSION}
          </span>,
          "Queue: no jobs",
        ]}
        trailing="No session open"
      />
      {toast && (
        <div
          style={{
            position: "fixed",
            left: "var(--space-7)",
            bottom: "calc(var(--app-status-h) + var(--space-7))",
            zIndex: 50,
          }}
        >
          <Toast
            tone={toast.tone}
            title={toast.title}
            message={toast.message}
            onClose={() => setToast(null)}
          />
        </div>
      )}
    </div>
  );
}

/** The page head and body of the active screen fill the grid's middle two rows. */
function Screen({ route, hasProfile }: { route: RouteId | null; hasProfile: boolean }) {
  switch (route) {
    case "sessions":
      return <SessionsScreen />;
    case "record":
      return <RecordScreen hasProfile={hasProfile} />;
    case "queue":
      return <QueueScreen />;
    case "review":
      return <ReviewScreen />;
    case "system":
      return <SystemScreen hasProfile={hasProfile} />;
    case "settings":
      return <SettingsScreen />;
    default:
      return (
        <>
          <div />
          <div />
        </>
      );
  }
}
