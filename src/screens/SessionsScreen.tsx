import { useMemo, useState } from "react";
import programme from "../../docs/programme.json";
import { Button, SessionItem, Tabs } from "../ds";
import { MainWithSide, PageHead } from "../shell/layout";

// Sessions placeholder: the STELMAR 2027 programme by day, seeded from docs/programme.json.
// Status and mode per session arrive with the recorder (Phase 1) and the Start Session dialog
// (Phase 3).

interface ProgrammeSession {
  id: string;
  day: string;
  date: string;
  start: string;
  end: string;
  kind: string | null;
  title: string;
  venue: string;
}

const SESSIONS = (programme as { sessions: ProgrammeSession[] }).sessions;

/** "2027-02-08" -> "Mon 8 Feb" in IST. */
function dayLabel(date: string): string {
  return new Intl.DateTimeFormat("en-IN", {
    weekday: "short",
    day: "numeric",
    month: "short",
    timeZone: "Asia/Kolkata",
  }).format(new Date(`${date}T12:00:00+05:30`));
}

export function SessionsScreen() {
  const days = useMemo(() => {
    const seen = new Map<string, string>();
    for (const s of SESSIONS) if (!seen.has(s.day)) seen.set(s.day, s.date);
    return [...seen.entries()].map(([day, date]) => ({ value: day, label: day, meta: dayLabel(date) }));
  }, []);
  const [day, setDay] = useState(days[0]?.value ?? "");
  const rows = SESSIONS.filter((s) => s.day === day);
  return (
    <>
      <PageHead
        overline="STELMAR 2027 · Programme"
        title="Sessions"
        meta={<span>Times in IST · the programme is tentative</span>}
        controls={
          <Button iconLeft="plus" disabled>
            New Session
          </Button>
        }
      />
      <MainWithSide>
        <Tabs size="lg" value={day} onChange={setDay} items={days} />
        <div style={{ borderTop: "var(--border-w) solid var(--ink-900)", marginTop: 20, maxWidth: 960 }}>
          {rows.map((s) => (
            <SessionItem
              key={s.id}
              start={s.start}
              end={s.end}
              kind={s.kind}
              title={s.title}
              room={s.venue}
            />
          ))}
        </div>
      </MainWithSide>
    </>
  );
}
