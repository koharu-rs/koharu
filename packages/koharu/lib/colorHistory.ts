'use client'

import { create } from 'zustand'
import { persist } from 'zustand/middleware'

/** The last eight colours chosen in a colour well or picked from the canvas, newest first, as Clip
 * Studio Paint's Color History palette keeps them. Every well shares them, and the interface keeps
 * them in its browser profile, as it does the favorite fonts. */
export const useColorHistory = create<{ colors: string[] }>()(
  persist(() => ({ colors: [] as string[] }), { name: 'color-history' }),
)

/** Puts an uppercase hex colour, as `toHex` writes it, at the front of the history. */
export function rememberColor(color: string): void {
  useColorHistory.setState(({ colors }) => ({
    colors: [color, ...colors.filter((entry) => entry !== color)].slice(0, 8),
  }))
}
