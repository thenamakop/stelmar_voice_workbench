import { useCallback, useEffect, useState } from "react";
import { Badge, Button, Dialog, Toast } from "./ds";
import type { ToastTone } from "./ds";
import { hasCapabilityProfile } from "./ipc";
import type { RecoveredSession } from "./ipc";
import { clock, gigabytes, istDateTime } from "./recording/format";
import { useRecorder } from "./recording/useRecorder";
import type { Recorder } from "./recording/useRecorder";
import { initialRoute, type RouteId } from "./routes";
import { QueueScreen } from "./screens/QueueScreen";
import { RecordScreen } from "./screens/RecordScreen";
import { ReviewScreen } from "./screens/ReviewScreen";
import { SessionsScreen } from "./screens/SessionsScreen";
import { SettingsScreen } from "./screens/SettingsScreen";
import { SystemScreen, systemStatus } from "./screens/SystemScreen";
import { useCapability } from "./system/useCapability";
import { machineLabel } from "./system/view";
import type { CapabilityState } from "./system/useCapability";
import { Note } from "./shell/layout";
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
  const notify = useCallback((t: ToastState) => setToast(t), []);
  const recorder = useRecorder(notify);
  const capability = useCapability(recorder.subscribeCheck, notify);

  useEffect(() => {
    let cancelled = false;
    hasCapabilityProfile()
      .then((found) => {
        if (cancelled) return;
        setHasProfile(found);
        setRoute((r) => r ?? initialRoute(found));
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setHasProfile(false);
        setRoute((r) => r ?? initialRoute(false));
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

  const recordingSession = recorder.snapshot?.session ?? null;
  // A session that starts or resumes (also after the display restarted) brings up Record.
  const sessionDir = recordingSession?.dir ?? null;
  const [shownSession, setShownSession] = useState<string | null>(null);
  if (sessionDir !== shownSession) {
    setShownSession(sessionDir);
    if (sessionDir) setRoute("record");
  }

  const cap = capability.view?.capability ?? null;
  const profileKnown = capability.view !== null || hasProfile !== null;
  const profileSaved = capability.view ? cap !== null : !!hasProfile;
  const profileText = !profileKnown
    ? "Checking…"
    : capability.running
      ? "System check running"
      : cap
        ? machineLabel(cap)
        : profileSaved
          ? "System check saved"
          : "System check not run";
  const sys = route === "system" ? systemStatus(capability) : null;
  const recovering = recorder.recovery[0] ?? null;

  return (
    <div
      style={{
        height: "100%",
        display: "grid",
        gridTemplateRows: "var(--app-bar-h) auto 1fr var(--app-status-h)",
        overflow: "hidden",
      }}
    >
      <TopBar active={route} onNavigate={setRoute} machineLabel={profileText} machineMono={!!cap} />
      <Screen route={route} hasProfile={profileSaved} recorder={recorder} notify={notify} capability={capability} onNavigate={setRoute} />
      <StatusBar
        items={[
          <span key="v" style={{ fontFamily: "var(--font-mono)" }}>
            v{APP_VERSION}
          </span>,
          ...(sys ? sys.items : []),
          recorder.snapshot?.recorder?.disk_free_bytes != null
            ? `Queue: no jobs · disk ${gigabytes(recorder.snapshot.recorder.disk_free_bytes)} free`
            : "Queue: no jobs",
        ]}
        trailing={sys && sys.trailing ? sys.trailing : recordingSession ? recordingSession.dir : "No session open"}
        trailingMono={!!(sys && sys.trailing) || !!recordingSession}
      />
      {recovering && !recordingSession && (
        <RecoveryDialog
          session={recovering}
          busy={recorder.busy}
          onAction={(action) => void recorder.recover(recovering.dir, action)}
        />
      )}
      {toast && (
        <div
          style={{
            position: "fixed",
            left: "var(--space-7)",
            bottom: "calc(var(--app-status-h) + var(--space-7))",
            zIndex: 1100,
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

/** Offered at start-up for each session a crash left open. */
function RecoveryDialog({
  session,
  busy,
  onAction,
}: {
  session: RecoveredSession;
  busy: boolean;
  onAction: (action: "resume" | "close") => void;
}) {
  const repaired = session.repairs.filter((r) => !r.already_valid).length;
  return (
    <Dialog
      open
      eyebrow="Recovered after an unexpected exit"
      title="Resume This Session?"
      width={560}
      footer={
        <>
          <Button variant="secondary" disabled={busy} onClick={() => onAction("close")}>
            Close Session
          </Button>
          <Button iconLeft="circle" disabled={busy} onClick={() => onAction("resume")}>
            Resume Recording
          </Button>
        </>
      }
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        <div style={{ fontSize: 16, fontWeight: "var(--fw-semibold)", color: "var(--ink-900)" }}>
          {session.title}
        </div>
        <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
          <span style={{ fontFamily: "var(--font-mono)", fontSize: 13, color: "var(--text-secondary)" }}>
            {istDateTime(session.opened_at)}
          </span>
          {session.hall && <Badge>{session.hall}</Badge>}
          <span style={{ fontFamily: "var(--font-mono)", fontSize: 13, color: "var(--text-secondary)" }}>
            {clock(session.audio_ms)}
          </span>
          <Badge>
            {session.parts} part{session.parts === 1 ? "" : "s"}
          </Badge>
        </div>
        <Note>
          The audio recorded before the app stopped is safe
          {repaired > 0 ? ` (${repaired} part header${repaired === 1 ? "" : "s"} repaired)` : ""}. Resume
          continues the same session in a new part; Close finishes it as it is.
        </Note>
      </div>
    </Dialog>
  );
}

/** The page head and body of the active screen fill the grid's middle two rows. */
function Screen({
  route,
  hasProfile,
  recorder,
  notify,
  capability,
  onNavigate,
}: {
  route: RouteId | null;
  hasProfile: boolean;
  recorder: Recorder;
  notify: (t: ToastState) => void;
  capability: CapabilityState;
  onNavigate: (r: RouteId) => void;
}) {
  switch (route) {
    case "sessions":
      return <SessionsScreen />;
    case "record":
      return <RecordScreen hasProfile={hasProfile} recorder={recorder} capability={capability} />;
    case "queue":
      return <QueueScreen />;
    case "review":
      return <ReviewScreen />;
    case "system":
      return <SystemScreen capability={capability} onContinue={() => onNavigate("record")} />;
    case "settings":
      return <SettingsScreen recorder={recorder} notify={notify} />;
    default:
      return (
        <>
          <div />
          <div />
        </>
      );
  }
}
