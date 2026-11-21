'use client'

import {
  commands,
  type Change,
  type CharStyle,
  type Layer,
  type LayerId,
  type Op,
  type PageId,
  type TextStyle,
  type Value,
} from '@koharu/ipc/protocol'

import { findPart, isBlockLayer, isTextLayer, letteringOf, type Move, type Part } from './document'

/** The panels' edits, each one undo step through the scene's own `edit`, labelled by a key the
 * History panel translates. */

function edit(label: string, ops: Op[]): Promise<Change> {
  return commands.edit({ label, ops })
}

/** Shows, hides, or fades layers. */
export function setVisibility(
  layers: LayerId[],
  visible: boolean | null,
  opacity: number | null,
): Promise<Change> {
  const patch: { [key in string]: Value } = {}
  if (visible !== null) patch.visible = visible
  if (opacity !== null) patch.opacity = opacity
  return edit(
    'set_visibility',
    layers.map((layer) => ({ op: 'update_layer', layer, patch })),
  )
}

/** Shows or hides a block's part: a hidden mask stops limiting the clean-up, and a hidden clean-up
 * shows the original. */
export function setPartVisibility(block: LayerId, part: Part, visible: boolean): Promise<Change> {
  return edit('set_visibility', [
    { op: 'update_layer', layer: block, patch: { [part]: { visible } } },
  ])
}

export function setLocked(layers: LayerId[], locked: boolean): Promise<Change> {
  return edit(
    'set_locked',
    layers.map((layer) => ({ op: 'update_layer', layer, patch: { locked } })),
  )
}

export function deleteLayers(layers: LayerId[]): Promise<Change> {
  return edit(
    'delete_layers',
    layers.map((layer) => ({ op: 'remove_layer', layer })),
  )
}

/** What Delete does when the page has the keys: deletes the selected layers, skipping a block's
 * selected part, which goes only with its block; the eraser clears a clean-up, and hiding a mask
 * stops it limiting the clean-up. */
export async function deleteSelection(selected: LayerId[], layers: Layer[]): Promise<void> {
  const whole = selected.filter((id) => !findPart(layers, id))
  if (whole.length) await deleteLayers(whole)
}

/** Moves layers in one undo step, each under its parent, or to the page's top level, at its
 * index counted after it's taken out, in order, as a drop in the Layers panel places them. */
export function moveLayers(moves: Move[]): Promise<Change> {
  return edit(
    'move_layer',
    moves.map(({ layer, parent, index }) => ({ op: 'move_layer', layer, parent, index })),
  )
}

/** A text layer's text, or a block's translation. */
export function setText(layer: LayerId, text: string): Promise<Change> {
  return edit('set_text', [{ op: 'set_text', layer, text }])
}

/** A block's source text, as OCR would have read it. */
export function setSourceText(layer: LayerId, text: string): Promise<Change> {
  return edit('set_source_text', [
    { op: 'update_layer', layer, patch: { source: { content: text } } },
  ])
}

/** Merges `style`, a `TextStyle` patch, into each layer's lettering: a text's own style, and a
 * block's translation's, which is what it draws. A character value it sets is every character's,
 * as Photoshop styles a selected type layer, so the characters marked with their own drop it. */
export function setStyle(layers: Layer[], style: TextStyle): Promise<Change> {
  const keys = Object.keys(style.chars ?? {}) as (keyof CharStyle)[]
  return edit(
    'set_style',
    layers
      .filter((layer) => isTextLayer(layer) || isBlockLayer(layer))
      .flatMap((layer): Op[] => {
        const patch: Value = isBlockLayer(layer) ? { translation: { style } } : { style }
        const restyle: Op = { op: 'update_layer', layer: layer.id, patch }
        const spans = letteringOf(layer)?.content ?? []
        const marked = spans.some((span) => keys.some((key) => span.attributes?.[key] != null))
        if (!marked) return [restyle]
        const unmarked = spans.map(({ insert, attributes }) => ({
          insert,
          attributes: Object.fromEntries(
            Object.entries(attributes ?? {}).filter(
              ([key]) => !keys.includes(key as keyof CharStyle),
            ),
          ),
        }))
        return [restyle, { op: 'set_rich_text', layer: layer.id, spans: unmarked }]
      }),
  )
}

/** A JSON Merge Patch of a layer's values, for the Inspector's frame, fit, anchor, direction, and
 * language. */
export function updateLayer(layer: LayerId, patch: Value): Promise<Change> {
  return edit('update_layer', [{ op: 'update_layer', layer, patch }])
}

export function deletePages(pages: PageId[]): Promise<Change> {
  return edit(
    'delete_pages',
    pages.map((page) => ({ op: 'remove_page', page })),
  )
}

/** Moves a page to `index`, counted after it's taken out. */
export function movePage(page: PageId, index: number): Promise<Change> {
  return edit('move_page', [{ op: 'move_page', page, index }])
}
