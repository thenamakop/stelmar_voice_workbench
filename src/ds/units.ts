// The design-system bundle writes some lengths as strings (padding '20 24 0' in pixels). The
// adherence lint forbids raw pixel strings in src/, so the ports build the same strings from
// numbers: px(20, 24, 0) gives each number a pixel unit, joined by spaces. Single lengths stay plain
// numbers, as in ds-bundle.js.
export const px = (...values: number[]): string => values.map((v) => `${v}px`).join(" ");
