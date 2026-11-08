import type { CSSProperties, HTMLAttributes } from "react";

// Ported from design/reference/ds-bundle.js (components/core/Icon.jsx). Same CSS-mask technique,
// but the SVGs are local files copied from lucide-static@0.460.0 by scripts/copy-icons.mjs
// instead of the unpkg CDN (offline app).
const BASE = `${import.meta.env.BASE_URL}icons/`;

export interface IconProps extends Omit<HTMLAttributes<HTMLSpanElement>, "color"> {
  name: string;
  size?: number;
  color?: string;
  label?: string;
  style?: CSSProperties;
}

export function Icon({ name, size = 18, color = "currentColor", label, style, ...rest }: IconProps) {
  const url = "url(" + BASE + name + ".svg) center/contain no-repeat";
  return (
    <span
      role={label ? "img" : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
      {...rest}
      style={{
        display: "inline-block",
        width: size,
        height: size,
        flex: "none",
        backgroundColor: color,
        WebkitMask: url,
        mask: url,
        ...style,
      }}
    />
  );
}
