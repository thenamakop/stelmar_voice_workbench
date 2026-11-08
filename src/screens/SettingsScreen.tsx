import { Card, Input, Select, Switch } from "../ds";
import { MainWithSide, Overline, PageHead } from "../shell/layout";

// Settings placeholder. Each setting is wired in the phase that needs it.
export function SettingsScreen() {
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
              disabled
              hint="Chosen in a later update"
            />
            <Select label="Backup drive" placeholder="Not set" options={[]} disabled />
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
