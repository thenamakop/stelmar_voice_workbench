import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { Badge, Button, Icon, ProgressBar, Tabs } from ".";
import { px } from "./units";

describe("px", () => {
  it("joins numbers into a px length list", () => {
    expect(px(20, 24, 0)).toBe("20px 24px 0px");
    expect(px(6)).toBe("6px");
  });
});

describe("Icon", () => {
  it("masks a local lucide SVG, never a CDN URL", () => {
    const { container } = render(<Icon name="bookmark" />);
    const span = container.querySelector("span");
    const mask = span?.style.mask ?? "";
    expect(mask).toContain("/icons/bookmark.svg");
    expect(mask).not.toMatch(/https?:/);
    expect(span?.getAttribute("aria-hidden")).toBe("true");
  });

  it("is an image with a name when labelled", () => {
    render(<Icon name="info" label="Information" />);
    expect(screen.getByRole("img", { name: "Information" })).toBeTruthy();
  });
});

describe("Button", () => {
  it("keeps the design-system sizes and disabled colours", () => {
    render(
      <Button size="sm" disabled>
        Start Session
      </Button>,
    );
    const button = screen.getByRole("button", { name: "Start Session" }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    expect(button.style.height).toBe("32px");
    expect(button.style.padding).toBe("0px 12px");
    expect(button.style.color).toBe("var(--text-disabled)");
  });
});

describe("Badge", () => {
  it("maps tones to status tokens", () => {
    render(<Badge tone="success">Offline</Badge>);
    const badge = screen.getByText("Offline");
    expect(badge.style.color).toBe("var(--status-success-fg)");
    expect(badge.style.background).toBe("var(--status-success-bg)");
  });
});

describe("Tabs", () => {
  it("marks the active tab and underlines it in orange", () => {
    render(<Tabs value="b" items={["a", "b"]} />);
    const tab = screen.getByRole("tab", { name: "b" });
    expect(tab.getAttribute("aria-selected")).toBe("true");
    expect(tab.style.borderBottom).toContain("var(--orange-600)");
  });
});

describe("ProgressBar", () => {
  it("clamps and reports progress", () => {
    const { rerender } = render(<ProgressBar value={0.74} label="Part 4" />);
    expect(screen.getByRole("progressbar", { name: "Part 4" }).getAttribute("aria-valuenow")).toBe("74");
    rerender(<ProgressBar value={3} label="Part 4" />);
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe("100");
    rerender(<ProgressBar value={Number.NaN} label="Part 4" />);
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe("0");
  });
});
