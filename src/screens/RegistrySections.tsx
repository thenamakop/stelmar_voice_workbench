import { useEffect, useState } from "react";
import { Badge, Icon, type BadgeTone } from "../ds";
import { px } from "../ds/units";
import { type EngineDevice, type EngineRow, type ModelRow, type RegistrySnapshot, listEngines } from "../ipc";
import { Note, Section, Table } from "../shell/layout";

// The Engines and Models sections of the System screen (Phase 2): which speech-engine builds this
// computer can run, with the devices each found, and which model files are installed and whether
// their checksums match. Built from the same Section, Table and Badge pieces as the rest of the screen.

/** Decimal gigabytes or megabytes, as file explorers show them. */
export function formatBytes(bytes: number): string {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  if (bytes >= 1e6) return `${(bytes / 1e6).toFixed(0)} MB`;
  return `${Math.max(0, Math.round(bytes / 1e3))} kB`;
}

const ENGINE_BADGE: Record<EngineRow["state"], [BadgeTone, string]> = {
  checking: ["neutral", "Checking…"],
  ok: ["success", "Ready"],
  cpu_mismatch: ["warning", "Needs a newer CPU"],
  failed: ["danger", "Failed"],
  timeout: ["danger", "No answer"],
};

const MODEL_BADGE: Record<ModelRow["state"], [BadgeTone, string]> = {
  checking: ["neutral", "Checking…"],
  ok: ["success", "Verified"],
  missing: ["warning", "Missing"],
  size_mismatch: ["danger", "Wrong size"],
  hash_mismatch: ["danger", "Checksum differs"],
  no_checksum: ["warning", "No checksum"],
};

const DEVICE_KIND: Record<EngineDevice["kind"], string> = {
  cpu: "CPU",
  gpu: "graphics card",
  igpu: "integrated graphics",
  accel: "accelerator",
  other: "device",
};

function describeDevice(d: EngineDevice): string {
  if (d.kind === "cpu") return `CPU · ${d.description}`;
  const parts = [d.adapter_name ?? d.description, DEVICE_KIND[d.kind]];
  if (d.shared_memory_bytes !== null) parts.push(`shares ${formatBytes(d.shared_memory_bytes)} of RAM`);
  else parts.push(formatBytes(d.total_memory_bytes));
  if (d.driver_version) parts.push(`driver ${d.driver_version}`);
  return parts.join(" · ");
}

function engineDetail(e: EngineRow): string {
  if (e.state === "ok") {
    const gpus = e.devices.filter((d) => d.kind !== "cpu");
    return gpus.length === 0 ? "CPU only" : gpus.map(describeDevice).join("; ");
  }
  return e.message ?? "";
}

export function EnginesSection({ registry }: { registry: RegistrySnapshot | null }) {
  const engines = registry?.engines ?? [];
  return (
    <Section title="Speech engines">
      <Table
        headers={["Build", "File", "Devices", "Result"]}
        rows={
          engines.length > 0
            ? engines.map((e) => {
                const [tone, label] = ENGINE_BADGE[e.state];
                return [
                  { content: e.variant, strong: true, mono: true },
                  { content: `${e.file} · ${formatBytes(e.bytes)}`, mono: true },
                  { content: engineDetail(e) },
                  { content: <Badge tone={tone}>{label}</Badge> },
                ];
              })
            : [
                [
                  { content: registry?.engines_done ? "None found" : "Looking…", strong: true },
                  { content: "" },
                  {
                    content: registry?.engines_done
                      ? "Recording works without an engine. Sessions are transcribed later, on a computer that has one."
                      : "",
                  },
                  {
                    content: (
                      <Badge tone={registry?.engines_done ? "warning" : "neutral"}>
                        {registry?.engines_done ? "No engine" : "Checking…"}
                      </Badge>
                    ),
                  },
                ],
              ]
        }
      />
      {registry?.engines_dir && (
        <Note style={{ marginTop: 8 }}>
          Each build runs in its own process, so one that cannot start on this computer never affects the
          recording. Looked in {registry.engines_dir}.
        </Note>
      )}
    </Section>
  );
}

export function ModelsSection({ registry }: { registry: RegistrySnapshot | null }) {
  const models = registry?.models ?? [];
  const checking = models.filter((m) => m.state === "checking").length;
  return (
    <Section title="Models">
      <Table
        headers={["Model", "Languages", "Roles", "Size", "Licence", "Checksum"]}
        rows={
          models.length > 0
            ? models.map((m) => {
                const [tone, label] = MODEL_BADGE[m.state];
                return [
                  { content: m.id, strong: true, mono: true },
                  { content: m.languages.map((l) => l.toUpperCase()).join(", ") },
                  { content: (m.validated_roles.length ? m.validated_roles : m.candidate_roles).join(", ") },
                  { content: formatBytes(m.bytes), mono: true },
                  { content: m.licence },
                  { content: <Badge tone={tone}>{label}</Badge> },
                ];
              })
            : [
                [
                  { content: registry?.models_done ? "No models" : "Looking…", strong: true },
                  { content: "" },
                  { content: registry?.manifest_error ?? "" },
                  { content: "" },
                  { content: "" },
                  {
                    content: (
                      <Badge tone={registry?.models_done ? "warning" : "neutral"}>
                        {registry?.models_done ? "Not installed" : "Checking…"}
                      </Badge>
                    ),
                  },
                ],
              ]
        }
      />
      <Note style={{ marginTop: 8 }}>
        {registry?.models_dir ? `Models folder: ${registry.models_dir}. ` : ""}A model whose checksum does not
        match is disabled and never used.
        {checking > 0 ? ` Checking ${checking} more…` : ""}
      </Note>
    </Section>
  );
}

/**
 * Shown on the Record screen while no speech engine can be used. Driven by the app state
 * (`speech_engine` from `list_engines`), which the screen reads when it loads, so it appears
 * whether or not the start-up notice was seen. Never shown while the engines are still being looked for.
 */
export function NoEngineBanner({ registry }: { registry: RegistrySnapshot | null }) {
  const kind = registry?.speech_engine;
  if (kind !== "none_found" && kind !== "none_usable") return null;
  return (
    <div
      role="status"
      aria-label="Speech engine"
      style={{
        display: "flex",
        gap: 10,
        alignItems: "center",
        marginTop: 12,
        padding: px(10, 14),
        borderRadius: "var(--radius-sm)",
        background: "var(--status-warning-bg)",
        color: "var(--status-warning-fg)",
        fontSize: 13,
        lineHeight: "var(--lh-small)",
      }}
    >
      <Icon name="alert-triangle" size={16} />
      <span>
        <b style={{ fontWeight: "var(--fw-semibold)" }}>
          {kind === "none_found" ? "No speech engine found." : "No speech engine can run on this computer."}
        </b>{" "}
        Recording is not affected. Sessions are recorded now and transcribed later, on a computer that has
        a working engine.
      </span>
    </div>
  );
}

/** Loads the registry and keeps it fresh until both checks are done. */
export function useRegistry(): RegistrySnapshot | null {
  const [registry, setRegistry] = useState<RegistrySnapshot | null>(null);
  useEffect(() => {
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const load = async () => {
      try {
        const snap = await listEngines();
        if (!alive) return;
        setRegistry(snap);
        if (!(snap.engines_done && snap.models_done)) timer = setTimeout(load, 1500);
      } catch {
        // The recording does not depend on this: try again later.
        if (alive) timer = setTimeout(load, 5000);
      }
    };
    void load();
    return () => {
      alive = false;
      if (timer) clearTimeout(timer);
    };
  }, []);
  return registry;
}
