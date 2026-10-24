'use client'

import { create } from 'zustand'

export const COLOR_HISTORY_LIMIT = 8
export const COLOR_HISTORY_STORAGE_KEY = 'koharu.color-history'

interface ColorHistoryStore {
  colors: string[]
  remember: (color: string) => void
}

export function nextColorHistory(history: string[], color: string): string[] {
  const normalized = normalizeHex(color)
  if (!normalized) return history
  if (history[0] === normalized) return history
  return [normalized, ...history.filter((entry) => entry !== normalized)].slice(
    0,
    COLOR_HISTORY_LIMIT,
  )
}

export const useColorHistory = create<ColorHistoryStore>((set, get) => ({
  colors: loadColorHistory(),
  remember: (color) => {
    const colors = nextColorHistory(get().colors, color)
    if (colors === get().colors) return
    set({ colors })
    persistColorHistory(colors)
  },
}))

export function rememberColor(color: string): void {
  useColorHistory.getState().remember(color)
}

function normalizeHex(value: string): string | null {
  const digits = value.startsWith('#') ? value.slice(1) : value
  const expanded =
    digits.length === 3 ? [...digits].map((digit) => digit.repeat(2)).join('') : digits
  if (!/^[0-9A-Fa-f]{6}$/.test(expanded)) return null
  return `#${expanded.toUpperCase()}`
}

function loadColorHistory(): string[] {
  if (typeof window === 'undefined') return []
  try {
    const parsed: unknown = JSON.parse(window.localStorage.getItem(COLOR_HISTORY_STORAGE_KEY) ?? '')
    if (!Array.isArray(parsed)) return []
    return parsed
      .reduce<string[]>((colors, item) => {
        if (typeof item !== 'string') return colors
        const normalized = normalizeHex(item)
        if (!normalized || colors.includes(normalized)) return colors
        colors.push(normalized)
        return colors
      }, [])
      .slice(0, COLOR_HISTORY_LIMIT)
  } catch {
    return []
  }
}

function persistColorHistory(colors: string[]): void {
  if (typeof window === 'undefined') return
  window.localStorage.setItem(COLOR_HISTORY_STORAGE_KEY, JSON.stringify(colors))
}
