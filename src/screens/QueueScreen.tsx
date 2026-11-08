import { Button, Card, Icon } from "../ds";
import { px } from "../ds/units";
import { MainWithSide, Note, PageHead } from "../shell/layout";

// Queue placeholder. The post-processing queue arrives in Phase 4.
export function QueueScreen() {
  return (
    <>
      <PageHead
        overline="Post-processing"
        title="Queue"
        meta={<span>Rolling and night passes run here at low priority</span>}
        controls={
          <Button variant="secondary" iconLeft="pause" disabled>
            Pause All
          </Button>
        }
      />
      <MainWithSide>
        <Card
          variant="muted"
          style={{ display: "flex", gap: 16, alignItems: "flex-start", maxWidth: 720, marginTop: 8 }}
        >
          <Icon name="list-checks" size={24} color="var(--ink-600)" />
          <div>
            <div
              style={{
                fontSize: 17,
                lineHeight: px(26),
                fontWeight: "var(--fw-semibold)",
                color: "var(--ink-900)",
              }}
            >
              No jobs
            </div>
            <Note>
              Each closed 10-minute part of a recording is queued here and transcribed in the background while
              recording continues.
            </Note>
          </div>
        </Card>
      </MainWithSide>
    </>
  );
}
