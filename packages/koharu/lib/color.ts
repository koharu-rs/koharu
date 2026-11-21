import { clampRgb, formatHex, rgb } from 'culori'

import type { Rgba } from '@koharu/ipc/protocol'

/** A CSS colour in any notation, a theme's `oklch()` included, as the canvas's straight RGBA from
 * 0 to 1, clamped into sRGB; black when it doesn't parse. */
export function toRgba(color: string): Rgba {
  const parsed = rgb(clampRgb(color))
  return parsed ? [parsed.r, parsed.g, parsed.b, parsed.alpha ?? 1] : [0, 0, 0, 1]
}

/** The canvas's RGBA as an uppercase hex colour, its alpha left out. */
export function toHex([r, g, b]: Rgba): string {
  return formatHex({ mode: 'rgb', r, g, b }).toUpperCase()
}
