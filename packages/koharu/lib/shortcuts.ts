import type { Tool } from '@koharu/ipc/protocol'

/** The editor's shortcuts, Photoshop's, as react-hotkeys-hook writes them: `mod` is ⌘ on macOS
 * and Ctrl elsewhere, and a letter is the one its key types in the user's layout. */
export const shortcuts = {
  undo: 'mod+z',
  redo: 'mod+shift+z',
  selectAll: 'mod+a',
  /** Of the area the marquee and the lasso marked. */
  deselect: 'mod+d',
  /** Fit on Screen. */
  fit: 'mod+0',
  delete: 'delete',
} as const

/** Each tool's key, Photoshop's for its counterpart: the Move tool's V, the Direct Selection
 * tool's A, the Type tool's T, the Brush's B, the Eraser's E, the Eyedropper's I, the Remove
 * tool's J, and the Hand's H. */
export const toolKeys: Record<Tool, string> = {
  select: 'v',
  direct_select: 'a',
  marquee: 'm',
  lasso: 'l',
  text: 't',
  draw: 'b',
  eraser: 'e',
  color_picker: 'i',
  remove: 'j',
  pan: 'h',
}

const macOS = typeof navigator !== 'undefined' && navigator.userAgent.includes('Macintosh')

/** Each key's name, in the order the platform writes them: macOS's symbols, run together, or
 * Windows' and Linux's names joined by `+`. */
const names: Record<string, string> = macOS
  ? { shift: '⇧', mod: '⌘', delete: '⌫' }
  : { mod: 'Ctrl', shift: 'Shift', delete: 'Del' }

/** A shortcut as a menu or a tooltip shows it, such as ⇧⌘Z or Ctrl+Shift+Z. */
export function shortcutLabel(hotkey: string): string {
  const order = Object.keys(names)
  const rank = (key: string) => (order.includes(key) ? order.indexOf(key) : order.length)
  return hotkey
    .split('+')
    .sort((left, right) => rank(left) - rank(right))
    .map((key) => names[key] ?? key.toUpperCase())
    .join(macOS ? '' : '+')
}
