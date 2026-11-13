import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import { Badge, Button, Checkbox, Dialog, Input, Radio, Select } from "../ds";
import { px } from "../ds/units";
import { sessionPlan } from "../ipc";
import type { HindiUrduScript, InputDevice, SessionPlan } from "../ipc";
import { format } from "../recording/format";
import { Note, Overline } from "../shell/layout";
import { LANG_NAME, recordPostText } from "../system/view";

// Start Session dialog per design/mockups/start-session.html. Mode, devices, models and the ready time
// come from the saved capability profile (src-tauri/src/syscheck, `session_plan`). The Hindi/Urdu
// script choice is not in the mockup: Hindi and Urdu are one spoken language and the organiser picks
// the script of the transcript (Devanagari by default, GATES 13 Nov 2026).
export interface StartChoice {
  title: string;
  hall: string;
  expected_languages: string[];
  mode: "record_post" | "live_post";
}

export interface StartSessionDialogProps {
  open: boolean;
  onClose: () => void;
  inputs: InputDevice[];
  deviceId: string;
  onDevice: (id: string) => void;
  busy: boolean;
  onStart: (choice: StartChoice) => void;
  /** Bumps when the saved profile changes, so the plan is read again. */
  profileKey: string;
  initialTitle?: string;
  initialHall?: string;
}

const MODE_GROUP = "start-mode";
const SCRIPT_GROUP = "start-script";

function Label({ children }: { children: ReactNode }) {
  return <Overline style={{ marginBottom: 6 }}>{children}</Overline>;
}

function ModeCard({
  selected,
  disabled,
  value,
  label,
  badge,
  children,
  onPick,
}: {
  selected: boolean;
  disabled?: boolean;
  value: string;
  label: string;
  badge?: ReactNode;
  children: ReactNode;
  onPick: () => void;
}) {
  return (
    <div
      style={{
        border: `${selected ? 2 : 1}px solid ${selected ? "var(--ink-900)" : "var(--border-hairline)"}`,
        borderRadius: "var(--radius-sm)",
        padding: px(selected ? 9 : 10, selected ? 13 : 14),
        background: disabled ? "var(--paper-50)" : "var(--paper-0)",
      }}
    >
      <Radio
        name={MODE_GROUP}
        value={value}
        checked={selected}
        disabled={disabled}
        onChange={onPick}
        label={
          <span style={{ display: "inline-flex", gap: 8, alignItems: "center" }}>
            {label}
            {badge}
          </span>
        }
        description={children}
      />
    </div>
  );
}

export function StartSessionDialog({
  open,
  onClose,
  inputs,
  deviceId,
  onDevice,
  busy,
  onStart,
  profileKey,
  initialTitle = "Unscheduled Session",
  initialHall = "",
}: StartSessionDialogProps) {
  const [title, setTitle] = useState(initialTitle);
  const [hall, setHall] = useState(initialHall);
  const [english, setEnglish] = useState(true);
  const [hindiUrdu, setHindiUrdu] = useState(true);
  const [script, setScript] = useState<HindiUrduScript>("devanagari");
  const [mode, setMode] = useState<"record_post" | "live_post">("record_post");
  const [plan, setPlan] = useState<SessionPlan | null>(null);
  const [planError, setPlanError] = useState<string | null>(null);

  const anyLanguage = english || hindiUrdu;

  useEffect(() => {
    if (!open || !anyLanguage) {
      return;
    }
    let cancelled = false;
    sessionPlan(english, hindiUrdu, script)
      .then((p) => {
        if (cancelled) return;
        setPlan(p);
        setPlanError(null);
        // A mode that stopped being available falls back to the default.
        if (p && !p.live.selectable) setMode("record_post");
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setPlan(null);
        setPlanError(err instanceof Error ? err.message : String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [open, english, hindiUrdu, script, anyLanguage, profileKey]);

  const langs = [...(english ? ["en"] : []), ...(hindiUrdu ? [script === "devanagari" ? "hi" : "ur"] : [])];
  const liveReason =
    plan?.live.reason ?? (plan === null ? "Not offered until the system check has run." : "Live captions are not offered.");
  const canStart = !busy && !!title.trim() && !!deviceId && anyLanguage && !(mode === "live_post" && !plan?.live.selectable);

  return (
    <Dialog
      open={open}
      onClose={onClose}
      eyebrow={hall.trim() ? `Start recording · ${hall.trim()}` : "Start recording"}
      title="Start Session"
      width={680}
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button
            iconLeft="circle"
            disabled={!canStart}
            onClick={() => onStart({ title: title.trim(), hall: hall.trim(), expected_languages: langs, mode })}
          >
            Start Recording
          </Button>
        </>
      }
    >
      <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
        <div style={{ display: "grid", gridTemplateColumns: "2fr 1fr", gap: 12 }}>
          <Input label="Session title" value={title} onChange={(e) => setTitle(e.target.value)} required />
          <Input label="Hall" placeholder="Seminar Hall B" value={hall} onChange={(e) => setHall(e.target.value)} />
        </div>
        <Select
          label="Audio input"
          value={deviceId}
          onChange={(e) => onDevice(e.target.value)}
          options={inputs.map((d) => ({
            value: d.id,
            label: `${d.name} · ${format(d.sample_rate, d.channels)}`,
          }))}
          placeholder="No input found"
        />
        <div>
          <Label>Languages expected</Label>
          <div style={{ display: "flex", gap: 20, flexWrap: "wrap" }}>
            <Checkbox label="English" checked={english} onChange={setEnglish} />
            <Checkbox label="Hindi / Urdu" checked={hindiUrdu} onChange={setHindiUrdu} />
          </div>
          {hindiUrdu && (
            <div style={{ marginTop: 10 }}>
              <div style={{ fontSize: 12, color: "var(--text-muted)", marginBottom: 6 }}>
                Hindi and Urdu are one spoken language. Write the transcript in:
              </div>
              <div style={{ display: "flex", gap: 24, flexWrap: "wrap" }}>
                <Radio
                  name={SCRIPT_GROUP}
                  value="devanagari"
                  checked={script === "devanagari"}
                  onChange={() => setScript("devanagari")}
                  label={
                    <span>
                      Devanagari <span lang="hi">(हिन्दी)</span>
                    </span>
                  }
                />
                <Radio
                  name={SCRIPT_GROUP}
                  value="nastaliq"
                  checked={script === "nastaliq"}
                  onChange={() => setScript("nastaliq")}
                  label={
                    <span>
                      Nastaliq <span lang="ur" dir="rtl">(اردو)</span>
                    </span>
                  }
                />
              </div>
            </div>
          )}
          <div style={{ fontSize: 12, lineHeight: px(18), color: "var(--text-muted)", marginTop: 8 }}>
            {anyLanguage
              ? english && hindiUrdu
                ? "Detected sentence by sentence between English and Hindi/Urdu, including switches mid-talk. Tick one language to lock it."
                : `Locked to ${english ? LANG_NAME.en : script === "devanagari" ? LANG_NAME.hi : LANG_NAME.ur}.`
              : "Tick at least one language."}
          </div>
        </div>
        <div>
          <Label>Mode</Label>
          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 12 }}>
            <ModeCard
              value="record_post"
              label="Record + post-process"
              badge={<Badge>Default</Badge>}
              selected={mode === "record_post"}
              onPick={() => setMode("record_post")}
            >
              {planError ?? recordPostText(plan)}
            </ModeCard>
            <ModeCard
              value="live_post"
              label="Live captions + post-process"
              selected={mode === "live_post"}
              disabled={!plan?.live.selectable}
              onPick={() => setMode("live_post")}
            >
              {plan?.live.selectable
                ? plan.live.via_override
                  ? "Allowed by the override on the System screen although the check measured this computer as too slow. Captions can lag. The live caption window is not in this version yet."
                  : "Captions appear while the talk is recorded. The live caption window is not in this version yet."
                : liveReason}
            </ModeCard>
          </div>
        </div>
        <div>
          <Label>Post-processing</Label>
          <div style={{ display: "grid", gridTemplateColumns: "auto 1fr", gap: px(6, 14), fontSize: 13, lineHeight: px(20), textAlign: "right" }}>
            <span style={{ color: "var(--text-muted)", textAlign: "left" }}>Devices</span>
            <b style={{ fontWeight: "var(--fw-semibold)" }}>{plan ? plan.devices : "Decided by the system check"}</b>
            <span style={{ color: "var(--text-muted)", textAlign: "left" }}>Models</span>
            <b style={{ fontWeight: "var(--fw-semibold)", fontFamily: "var(--font-mono)" }}>
              {plan && plan.models.length > 0
                ? plan.models.map((m) => `${m.model} (${m.lang})${m.quality_unverified ? "*" : ""}`).join(" · ")
                : "—"}
            </b>
          </div>
          {plan?.models.some((m) => m.quality_unverified) && (
            <Note style={{ marginTop: 6, fontSize: 12, color: "var(--text-muted)" }}>
              * Accuracy of this model for this language has not been measured on this project's test sets yet.
            </Note>
          )}
          {plan?.partial && (
            <Note style={{ marginTop: 6, fontSize: 12, color: "var(--text-muted)" }}>
              The system check was partial. Run the full check on the System screen for complete numbers.
            </Note>
          )}
        </div>
      </div>
    </Dialog>
  );
}
