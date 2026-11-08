import { useState } from "react";
import { Card, Icon, Input, Tabs } from "../ds";
import { px } from "../ds/units";
import { MainWithSide, Note, PageHead } from "../shell/layout";

// Review placeholder. The transcript editor arrives in Phase 6.
export function ReviewScreen() {
  const [tab, setTab] = useState("transcript");
  return (
    <>
      <PageHead
        overline="Review"
        title="Transcripts"
        meta={<span>Edit transcripts against the audio, then export</span>}
        controls={
          <Input iconLeft="search" placeholder="Search transcripts" disabled style={{ width: 280 }} />
        }
      />
      <MainWithSide>
        <Tabs
          value={tab}
          onChange={setTab}
          items={[
            { value: "transcript", label: "Transcript" },
            { value: "revisions", label: "Revisions" },
            { value: "speakers", label: "Speakers" },
            { value: "bookmarks", label: "Bookmarks" },
          ]}
        />
        <Card
          variant="muted"
          style={{ display: "flex", gap: 16, alignItems: "flex-start", maxWidth: 720, marginTop: 20 }}
        >
          <Icon name="file-text" size={24} color="var(--ink-600)" />
          <div>
            <div
              style={{
                fontSize: 17,
                lineHeight: px(26),
                fontWeight: "var(--fw-semibold)",
                color: "var(--ink-900)",
              }}
            >
              No transcripts yet
            </div>
            <Note>Transcripts appear here once a recorded session has been post-processed.</Note>
          </div>
        </Card>
      </MainWithSide>
    </>
  );
}
