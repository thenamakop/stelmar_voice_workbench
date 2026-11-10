import { useState } from "react";
import { Button, Card, Input, Select, Switch } from "../ds";
import type { ToastTone } from "../ds";
import type { Recorder } from "../recording/useRecorder";
import { MainWithSide, Note, Overline, PageHead } from "../shell/layout";

// Settings. Phase 1 wires the folders (recordings and backup); the rest is wired in the phase that
// needs it. Folders are typed as full paths: a folder picker would need a Tauri plugin, and the
// app ships none.

export interface SettingsScreenProps {
  recorder: Recorder;
  notify: (toast: { tone: ToastTone; title: string; message?: string }) => void;
}

export function SettingsScreen({ recorder, notify }: SettingsScreenProps) {
  const current = recorder.settings;
  // Edits in progress; null fields show the saved value.
  const [draft, setDraft] = useState<{ recordings: string | null; backup: string | null }>({
    recordings: null,
    backup: null,
  });
  const recordings = draft.recordings ?? current?.recordings_root ?? "";
  const backup = draft.backup ?? current?.backup_root ?? "";
  const setRecordings = (v: string) => setDraft((d) => ({ ...d, recordings: v }));
  const setBackup = (v: string) => setDraft((d) => ({ ...d, backup: v }));
  const recording = !!recorder.snapshot?.session;
  const changed =
    !!current &&
    (recordings !== current.recordings_root || (backup.trim() || null) !== (current.backup_root ?? null));

  const save = async () => {
    if (!current) return;
    const ok = await recorder.saveSettings({
      ...current,
      recordings_root: recordings.trim(),
      backup_root: backup.trim() || null,
    });
    if (ok) setDraft({ recordings: null, backup: null });
    if (ok) notify({ tone: "success", title: "Folders saved", message: "They apply from the next session." });
  };

  return (
    <>
      <PageHead
        overline="Settings"
        title="Settings"
        meta={<span>Folders, backup, glossary, captions and hotkeys</span>}
      />
      <MainWithSide>
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
            gap: "var(--space-5)",
            maxWidth: 960,
            marginTop: 8,
          }}
        >
          <Card style={{ display: "flex", flexDirection: "column", gap: 16 }}>
            <Overline>Folders</Overline>
            <Input
              label="Recordings folder"
              placeholder="D:\STELMAR-Transcripts"
              value={recordings}
              onChange={(e) => setRecordings(e.target.value)}
              disabled={!current}
              hint="Full path. Each session gets a dated folder here."
            />
            <Input
              label="Backup folder"
              placeholder="E:\STELMAR-Backup"
              value={backup}
              onChange={(e) => setBackup(e.target.value)}
              disabled={!current}
              hint="On a second drive. Every session is copied here while it records. Leave empty for none."
            />
            <div style={{ display: "flex", gap: 12, alignItems: "center" }}>
              <Button size="sm" disabled={!changed || recorder.busy} onClick={() => void save()}>
                Save Folders
              </Button>
              {recording && <Note>The session that is recording keeps its folders.</Note>}
            </div>
          </Card>
          <Card style={{ display: "flex", flexDirection: "column", gap: 16 }}>
            <Overline>Glossary</Overline>
            <Input
              label="Terms for every session"
              multiline
              rows={4}
              placeholder="STELMAR, BML Munjal University, speaker names"
              disabled
            />
          </Card>
          <Card style={{ display: "flex", flexDirection: "column", gap: 16 }}>
            <Overline>Scheduler</Overline>
            <Switch label="Pause post-processing on battery power" checked disabled />
            <Switch label="Run the night pass" checked disabled />
          </Card>
          <Card style={{ display: "flex", flexDirection: "column", gap: 16 }}>
            <Overline>Captions</Overline>
            <Select label="Caption size" options={["Standard", "Large"]} disabled />
          </Card>
        </div>
      </MainWithSide>
    </>
  );
}
