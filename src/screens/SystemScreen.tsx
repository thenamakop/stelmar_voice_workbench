import { Badge, Button, Switch } from "../ds";
import { px } from "../ds/units";
import { MainWithSide, Note, PageHead, Section, SideCard, Table } from "../shell/layout";
import { liveUnavailableReason } from "./liveReason";

// System screen placeholder per design/mockups/system-check.html. The system check itself arrives
// in Phase 3; until then the Run System Check button stays disabled.
export interface SystemScreenProps {
  hasProfile: boolean;
}

const HARDWARE = ["Processor", "Memory", "Graphics", "Storage and power", "Audio input"];

export function SystemScreen({ hasProfile }: SystemScreenProps) {
  return (
    <>
      <PageHead
        overline={hasProfile ? "System check" : "First launch · System check"}
        title={hasProfile ? "This Computer's System Check" : "Check This Computer Before the First Recording"}
        meta={
          <>
            <span>{hasProfile ? "A saved capability profile was found" : "Not run yet"}</span>
            <Badge>No tier</Badge>
          </>
        }
        controls={
          <Button iconLeft="refresh-cw" disabled>
            Run System Check
          </Button>
        }
      />
      <MainWithSide
        side={
          <>
            <SideCard title="Verdict">
              <div
                style={{
                  fontSize: 22,
                  lineHeight: px(30),
                  fontWeight: "var(--fw-bold)",
                  letterSpacing: "var(--ls-tight)",
                  color: "var(--ink-900)",
                  margin: px(0, 0, 6),
                }}
              >
                Not checked yet
              </div>
              <Note>
                The system check measures this computer and decides how each session is recorded and
                transcribed here. Live text stays off until the check clears it.
              </Note>
            </SideCard>
            <SideCard title="Overrides" stack>
              <Switch label="Process while recording" checked disabled />
              <Switch label="Pause on battery power" checked disabled />
              <Switch label="Allow live captions anyway" disabled />
              <Note style={{ fontSize: 12, lineHeight: px(18), color: "var(--text-muted)" }}>
                {liveUnavailableReason(hasProfile)}
              </Note>
            </SideCard>
          </>
        }
      >
        <Section title="Hardware" first>
          <Table
            headers={["Part", "Found", "Result"]}
            rows={HARDWARE.map((part) => [
              { content: part, strong: true },
              { content: "Not checked yet" },
              { content: <Badge>Pending</Badge> },
            ])}
          />
        </Section>
        <Section title="Speed test">
          <Table
            headers={["Model", "Role", "Devices", "Speed", "Memory", "Result"]}
            rows={[
              [
                { content: "No models measured" },
                { content: "" },
                { content: "" },
                { content: "" },
                { content: "" },
                { content: <Badge>Pending</Badge> },
              ],
            ]}
          />
        </Section>
      </MainWithSide>
    </>
  );
}
