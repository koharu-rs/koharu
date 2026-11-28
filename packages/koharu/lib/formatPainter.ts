'use client'

import { create } from 'zustand'

import { sharedChars } from '@/components/editor/TypeBar'
import type {
  CharKey,
  Layer,
  LayerId,
  Page,
  PainterPress,
  Text,
  TextStyle,
} from '@koharu/ipc/protocol'
import { toast } from '@koharu/ui/components/toast'

import { findLayer, letteringOf, plainText } from './document'
import i18n from './i18n'
import { pageKey, queryClient, readPage } from './queries'
import { pasteStyle } from './scene'
import { useKoharuStore } from './store'

/** What the Format Painter copies of a text's characters: all but how they stand in a vertical
 * column, which goes with the text's direction, as its box does. */
const COPIED: CharKey[] = [
  'family',
  'weight',
  'italic',
  'size',
  'fill',
  'stroke',
  'stroke_width',
  'emphasis',
]

/** A style the Format Painter holds: the `TextStyle` patch it pastes, the text it came from as
 * that text drew it, for the bar's preview, and a few of its words, to name it. */
export type Copied = { style: TextStyle; text: Text; words: string }

export const useFormatPainter = create<{ copied: Copied | null }>(() => ({ copied: null }))

/** Drops the style held, as Esc, the bar's ×, and another tool do. The Hand stays out of it, as
 * Space holds it for a moment and gives the Format Painter back. */
export function clearFormat(): void {
  useFormatPainter.setState({ copied: null })
}

useKoharuStore.subscribe((state, previous) => {
  if (state.tool !== previous.tool && !['format_painter', 'pan'].includes(state.tool)) clearFormat()
})

/** `text`'s style as a patch that makes another text look as it does: a value every character
 * shares, and empty where it sets none, so a target drops its own there and follows its style as
 * this one does. A value its characters don't share is left as the target has it. */
function copyOf(text: Text): TextStyle {
  const { chars, mixed } = sharedChars([text])
  return {
    chars: Object.fromEntries(
      COPIED.filter((key) => !mixed.includes(key)).map((key) => [key, chars[key] ?? null]),
    ),
    align: text.style?.align ?? null,
    line_height: text.style?.line_height ?? null,
    kernel: text.style?.kernel ?? null,
  }
}

/** Copies `layer`'s style. */
function copy(layer: Layer): void {
  const text = letteringOf(layer)
  if (!text) return
  const words = plainText(text).trim().replace(/\s+/g, ' ')
  useFormatPainter.setState({
    copied: {
      style: copyOf(text),
      text,
      words: words.length > 24 ? `${words.slice(0, 24)}…` : words,
    },
  })
}

/** Pastes the style held onto `layers`, in one undo step. */
export async function paint(layers: Layer[]): Promise<void> {
  const copied = useFormatPainter.getState().copied
  if (!copied || layers.length === 0) return
  try {
    await pasteStyle(layers, copied.style)
    await readPage()
  } catch (error) {
    toast.add({ type: 'error', title: i18n.t('formatPainter.failed'), description: String(error) })
  }
}

/** A press of the Format Painter on a text: copies its style when none is held or Alt was down,
 * and pastes the style held onto it otherwise. */
export function receivePainter({ layer, reload }: PainterPress): void {
  const page = queryClient.getQueryData<Page | null>(pageKey)
  const found = page && findLayer(page.layers, layer as LayerId)?.layer
  if (!found) return
  if (reload || !useFormatPainter.getState().copied) copy(found)
  else void paint([found])
}
