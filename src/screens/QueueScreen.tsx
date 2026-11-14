import { useState } from "react";
import { Badge, Button, Card, Dialog, Icon, Input, ProgressBar, Select } from "../ds";
import { px } from "../ds/units";
import type { ModelRow, QueueJob, RegistrySnapshot } from "../ipc";
import { clock } from "../recording/format";
import {
  KIND_LABEL,
  LANGUAGE_NAMES,
  STATE_BADGE,
  jobProgress,
  modelPairs,
  readyText,
  openJobs,
  percent,
  ramText,
  sessionLabel,
} from "../queue/view";
import type { QueueState } from "../queue/useQueue";
import { KeyValue, MainWithSide, Note, PageHead, SideCard } from "../shell/layout";
import type { CapabilityState } from "../system/useCapability";
import { useRegistry } from "./RegistrySections";

// Queue screen (Phase 4). No mockup exists; it is built from PLAN section 4 and section 9 with the same
// pieces as the other screens: a job row per session pass, a progress bar, a Badge for the state, a
// side card for the scheduler.

function minutesLeft(job: QueueJob, rtf: number): string | null {
  if (job.state === "done" || job.remaining_audio_ms <= 0) return null;
  const min = Math.max(1, Math.round((job.remaining_audio_ms / 60000) * rtf));
  return `about ${min} min left`;
}

/** Models that can decode this language, from the registry's verified models (the job's own model first). */
export function modelChoices(registry: RegistrySnapshot | null, lang: string, current: string): string[] {
  const ids = (registry?.models ?? [])
    .filter((m: ModelRow) => m.state === "ok" && m.languages.includes(lang))
    .map((m) => m.id);
  return [...new Set([current, ...ids])];
}

function JobCard({
  job,
  queue,
  onChange,
  speed,
}: {
  job: QueueJob;
  queue: QueueState;
  onChange: (job: QueueJob) => void;
  speed: number;
}) {
  const [tone, label] = STATE_BADGE[job.state];
  const fraction = jobProgress(job);
  const left = minutesLeft(job, speed);
  const waiting = job.state === "queued" || job.state === "running";
  return (
    <Card
      padding={px(16, 20)}
      style={{ marginBottom: 12 }}
      role="group"
      aria-label={`Job ${sessionLabel(job.session)}`}
    >
      <div style={{ display: "flex", alignItems: "center", gap: 10, flexWrap: "wrap" }}>
        <div
          style={{
            fontSize: "var(--fs-body)",
            lineHeight: px(24),
            fontWeight: "var(--fw-semibold)",
            color: "var(--ink-900)",
          }}
        >
          {sessionLabel(job.session)}
        </div>
        <Badge>{KIND_LABEL[job.kind]}</Badge>
        <Badge tone={tone} dot={job.state === "running"}>
          {label}
        </Badge>
        {job.state !== "done" && (
          <span style={{ marginLeft: "auto", display: "flex", gap: 8 }}>
            {job.state === "paused" ? (
              <Button size="sm" variant="secondary" iconLeft="play" onClick={() => void queue.resume(job.id)}>
                Resume
              </Button>
            ) : (
              <Button
                size="sm"
                variant="secondary"
                iconLeft="pause"
                disabled={job.state === "parked"}
                title={job.state === "parked" ? "Needs attention: see the message below" : undefined}
                onClick={() => void queue.pause(job.id)}
              >
                Pause
              </Button>
            )}
            <Button
              size="sm"
              variant="secondary"
              iconLeft="arrow-up"
              disabled={!waiting}
              onClick={() => void queue.moveUp(job.id)}
            >
              Move Up
            </Button>
            <Button size="sm" variant="secondary" iconLeft="replace" onClick={() => onChange(job)}>
              Change Model
            </Button>
          </span>
        )}
      </div>
      <div
        style={{ fontSize: 13, lineHeight: "var(--lh-small)", color: "var(--text-secondary)", marginTop: 4 }}
      >
        {modelPairs(job).map(([lang, id], k) => (
          <span key={lang}>
            {k > 0 ? " · " : ""}
            {lang}:{" "}
            <span style={{ fontFamily: "var(--font-mono)", fontSize: "var(--fs-caption)" }}>{id}</span>
          </span>
        ))}
        {modelPairs(job).length === 0 && "No model chosen"}
      </div>
      <div style={{ display: "flex", alignItems: "center", gap: 14, marginTop: 12 }}>
        <ProgressBar
          value={fraction}
          label={`${sessionLabel(job.session)} transcription`}
          style={{ flex: 1, maxWidth: 520 }}
        />
        <span style={{ fontSize: 13, color: "var(--ink-900)", minWidth: 40 }}>{percent(fraction)}</span>
        <span style={{ fontSize: 13, color: "var(--text-secondary)" }}>
          {job.units_committed} of {job.units_total} unit{job.units_total === 1 ? "" : "s"}
          {job.units_running > 0 ? ` · ${job.units_running} running` : ""}
          {left ? ` · ${left}` : ""}
          {job.state !== "done" && job.cursor_ms > 0 ? ` · done to ${clock(job.cursor_ms)}` : ""}
        </span>
      </div>
      {job.error && (
        <div
          role="alert"
          style={{
            display: "flex",
            gap: 8,
            alignItems: "flex-start",
            marginTop: 10,
            fontSize: 13,
            lineHeight: "var(--lh-small)",
            color: "var(--status-danger-fg)",
          }}
        >
          <Icon name="alert-triangle" size={16} />
          <span>{job.error}</span>
        </div>
      )}
    </Card>
  );
}

function ChangeModelDialog({
  job,
  registry,
  onClose,
  onApply,
}: {
  job: QueueJob | null;
  registry: RegistrySnapshot | null;
  onClose: () => void;
  onApply: (job: QueueJob, models: Record<string, string>) => void;
}) {
  const [chosen, setChosen] = useState<Record<string, string>>({});
  if (!job) return null;
  const value = (lang: string) => chosen[lang] ?? job.models_by_lang[lang] ?? "";
  return (
    <Dialog
      open
      onClose={onClose}
      eyebrow="Applies to the units not yet transcribed"
      title="Change Model"
      width={520}
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button
            onClick={() =>
              onApply(job, Object.fromEntries(Object.keys(job.models_by_lang).map((l) => [l, value(l)])))
            }
          >
            Apply
          </Button>
        </>
      }
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
        <div style={{ fontSize: 15, fontWeight: "var(--fw-semibold)", color: "var(--ink-900)" }}>
          {sessionLabel(job.session)}
        </div>
        {Object.keys(job.models_by_lang).map((lang) => (
          <Select
            key={lang}
            label={LANGUAGE_NAMES[lang] ?? lang}
            value={value(lang)}
            onChange={(e) => setChosen((c) => ({ ...c, [lang]: e.target.value }))}
            options={modelChoices(registry, lang, job.models_by_lang[lang] ?? "")}
          />
        ))}
        <Note>
          A unit that is running now is stopped and starts again with the new model. Segments already written
          stay as they are.
        </Note>
      </div>
    </Dialog>
  );
}

function ImportDialog({
  open,
  onClose,
  onImport,
}: {
  open: boolean;
  onClose: () => void;
  onImport: (folder: string) => Promise<boolean>;
}) {
  const [folder, setFolder] = useState("");
  const [busy, setBusy] = useState(false);
  return (
    <Dialog
      open={open}
      onClose={onClose}
      eyebrow="From another computer"
      title="Import a Session"
      width={560}
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button
            iconLeft="folder-input"
            disabled={busy || folder.trim() === ""}
            onClick={() => {
              setBusy(true);
              void onImport(folder.trim()).then((ok) => {
                setBusy(false);
                if (ok) {
                  setFolder("");
                  onClose();
                }
              });
            }}
          >
            Import
          </Button>
        </>
      }
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
        <Input
          label="Session folder"
          placeholder="D:\\STELMAR-Transcripts\\2027-02-08\\seminar-hall-b__parallel-track-sessions-i"
          value={folder}
          onChange={(e) => setFolder(e.target.value)}
        />
        <Note>
          The folder is checked (every audio part against its checksum), copied into this computer's
          recordings folder and transcribed like a recording that has just ended. The original is not changed.
        </Note>
      </div>
    </Dialog>
  );
}

export interface QueueScreenProps {
  queue: QueueState;
  capability: CapabilityState;
}

export function QueueScreen({ queue, capability }: QueueScreenProps) {
  const registry = useRegistry();
  const [changing, setChanging] = useState<QueueJob | null>(null);
  const [importOpen, setImportOpen] = useState(false);
  const snap = queue.snapshot;
  const jobs = snap?.jobs ?? [];
  const open = openJobs(snap);
  const done = jobs.filter((j) => j.state === "done");
  const sched = snap?.scheduler ?? null;
  const decision = capability.view?.capability?.decision ?? null;
  const operatorPaused = !!sched?.operator_paused;
  // The speed that turns remaining audio into minutes: the slowest measured speed among the job's models.
  const measured = capability.view?.capability?.measurements ?? [];
  const speedOf = (job: QueueJob) =>
    Math.max(
      0.01,
      ...Object.entries(job.models_by_lang).map(
        ([lang, id]) =>
          measured.find((m) => m.model === id && m.lang === lang && m.mode === "rolling")?.rtf ?? 1,
      ),
    );

  return (
    <>
      <PageHead
        overline="Post-processing"
        title="Queue"
        meta={
          <>
            <span>
              {snap ? readyText(snap.ready_text) : "Rolling and night passes run here at low priority"}
            </span>
            {sched?.paused && <Badge tone="warning">Paused</Badge>}
          </>
        }
        controls={
          <>
            <Button variant="secondary" iconLeft="folder-input" onClick={() => setImportOpen(true)}>
              Import Session
            </Button>
            {operatorPaused ? (
              <Button variant="secondary" iconLeft="play" onClick={() => void queue.resume(null)}>
                Resume All
              </Button>
            ) : (
              <Button
                variant="secondary"
                iconLeft="pause"
                disabled={open.length === 0}
                onClick={() => void queue.pause(null)}
              >
                Pause All
              </Button>
            )}
          </>
        }
      />
      <MainWithSide
        side={
          <>
            <SideCard title="Scheduler">
              <KeyValue
                rows={[
                  [
                    "State",
                    sched ? (sched.paused ? "Held" : open.length > 0 ? "Working" : "Idle") : "Not started",
                  ],
                  ["Engines", decision?.config ? decision.config.label : "Not chosen yet"],
                  [
                    "Threads",
                    decision?.config
                      ? String(Math.max(1, decision.config.cpu_threads - (sched?.fewer_threads ?? 0)))
                      : "Not chosen yet",
                  ],
                  ["Free memory", sched ? ramText(sched.free_ram_bytes) : "Not read yet"],
                ]}
              />
              {sched?.paused && (
                <div
                  role="status"
                  style={{ display: "flex", gap: 8, alignItems: "flex-start", marginTop: 10 }}
                >
                  <Icon name="pause" size={14} color="var(--status-warning-fg)" />
                  <Note style={{ color: "var(--status-warning-fg)" }}>{sched.paused}</Note>
                </div>
              )}
              {(sched?.fewer_threads ?? 0) > 0 && (
                <Note style={{ marginTop: 8 }}>
                  One thread fewer since the recorder reported a dropout. It stays until the machine is
                  restarted.
                </Note>
              )}
            </SideCard>
            {sched && sched.profile_changes.length > 0 && (
              <SideCard title="This computer changed">
                <Note>
                  Safe defaults are in force: {sched.profile_changes.join("; ")}. Run the system check on the
                  System screen.
                </Note>
              </SideCard>
            )}
            <SideCard title="Recent">
              {sched && sched.log.length > 0 ? (
                <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                  {sched.log.slice(0, 8).map((line, i) => (
                    <div
                      key={i}
                      style={{
                        fontSize: "var(--fs-caption)",
                        lineHeight: "var(--lh-caption)",
                        color: "var(--text-secondary)",
                        overflowWrap: "anywhere",
                      }}
                    >
                      {line}
                    </div>
                  ))}
                </div>
              ) : (
                <Note>Nothing yet.</Note>
              )}
            </SideCard>
          </>
        }
      >
        {queue.unavailable && !snap && (
          <Card variant="muted" style={{ display: "flex", gap: 16, alignItems: "flex-start", maxWidth: 720 }}>
            <Icon name="info" size={24} color="var(--ink-600)" />
            <div>
              <div
                style={{
                  fontSize: "var(--fs-h4)",
                  lineHeight: "var(--lh-h4)",
                  fontWeight: "var(--fw-semibold)",
                  color: "var(--ink-900)",
                }}
              >
                Post-processing is not available
              </div>
              <Note>{queue.unavailable}</Note>
            </div>
          </Card>
        )}
        {!queue.unavailable && jobs.length === 0 && (
          <Card variant="muted" style={{ display: "flex", gap: 16, alignItems: "flex-start", maxWidth: 720 }}>
            <Icon name="list-checks" size={24} color="var(--ink-600)" />
            <div>
              <div
                style={{
                  fontSize: "var(--fs-h4)",
                  lineHeight: "var(--lh-h4)",
                  fontWeight: "var(--fw-semibold)",
                  color: "var(--ink-900)",
                }}
              >
                No jobs
              </div>
              <Note>
                Each closed 10-minute part of a recording is queued here and transcribed in the background
                while recording continues.
              </Note>
            </div>
          </Card>
        )}
        {open.map((j) => (
          <JobCard key={j.id} job={j} queue={queue} onChange={setChanging} speed={speedOf(j)} />
        ))}
        {done.length > 0 && (
          <>
            <div style={{ fontSize: "var(--fs-caption)", color: "var(--text-muted)", margin: px(8, 0, 8) }}>
              Finished
            </div>
            {done.map((j) => (
              <JobCard key={j.id} job={j} queue={queue} onChange={setChanging} speed={speedOf(j)} />
            ))}
          </>
        )}
      </MainWithSide>
      <ChangeModelDialog
        key={changing?.id ?? "none"}
        job={changing}
        registry={registry}
        onClose={() => setChanging(null)}
        onApply={(job, models) => {
          void queue.changeModel(job.id, models).then((ok) => {
            if (ok) setChanging(null);
          });
        }}
      />
      <ImportDialog open={importOpen} onClose={() => setImportOpen(false)} onImport={queue.importFolder} />
    </>
  );
}
