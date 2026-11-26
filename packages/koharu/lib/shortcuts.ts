import { useQuery } from '@tanstack/react-query'

import { commands, type Tool } from '@koharu/ipc/protocol'

/** The editor's shortcuts, Photoshop's, as react-hotkeys-hook writes them: `mod` is ⌘ on macOS
 * and Ctrl elsewhere, a letter is the one its key types in the user's layout, and a digit is its
 * key's wherever the layout puts it, as Ctrl+0 is on AZERTY's unshifted 0. */
export const shortcuts = {
  undo: 'mod+z',
  redo: 'mod+shift+z',
  selectAll: 'mod+a',
  /** Of the area the marquee and the lasso marked. */
  deselect: 'mod+d',
  /** Fit on Screen. */
  fit: 'mod+0',
  /** Lightroom's Before and After, on the key standing for US `\`: see `useKeyboard`. */
  original: 'Backslash',
  /** Photoshop's Layers panel. */
  layers: 'f7',
  delete: 'delete',
  /** Photoshop's Bold and Italic, which fake what the font lacks. */
  bold: 'mod+shift+b',
  italic: 'mod+shift+i',
} as const

/** Each tool's key, Photoshop's for its counterpart: the Move tool's V, the Direct Selection
 * tool's A, the Type tool's T, the Frame tool's K, the Brush's B, the Eraser's E, the Eyedropper's
 * I, the Remove tool's J, and the Hand's H. */
export const toolKeys: Record<Tool, string> = {
  select: 'v',
  direct_select: 'a',
  marquee: 'm',
  lasso: 'l',
  text: 't',
  block: 'k',
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

/** What the user's layout makes of a shortcut on a US punctuation key, named by its
 * `KeyboardEvent.code`: the key standing for it, and what that key types. Windows apps take a
 * shortcut's key by its virtual-key code, which a layout may give another key, such as Spanish
 * `º` for `\`, and the desktop says which; elsewhere the key keeps its US place. A label reads
 * the layout again as it shows, so it names the key that works after a switch of layouts too. */
export function useKeyboard() {
  const { data } = useQuery({
    queryKey: ['keyboard'],
    queryFn: () => commands.getKeyboard(),
    staleTime: 0,
  })
  return {
    /** The code of the key standing for the US key at `code`. */
    place: (code: string) => data?.[code]?.code ?? code,
    /** What that key types, as a label shows it, once the layout is read. */
    label: (code: string) => data?.[code]?.label.toUpperCase(),
  }
}
