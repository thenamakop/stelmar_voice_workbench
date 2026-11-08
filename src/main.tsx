import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

// Design tokens, unchanged, in the order the design system loads them; then the app additions
// and the offline fonts. Vite bundles the font files from design/fonts/.
import "../design/tokens/colors.css";
import "../design/tokens/typography.css";
import "../design/tokens/spacing.css";
import "../design/tokens/effects.css";
import "../design/tokens/base.css";
import "../design/tokens/app-additions.css";
import "../design/fonts.css";
import "./app.css";

import { App } from "./App";

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}
