'use client'

import {
  commands,
  type Change,
  type Layer,
  type LayerId,
  type Op,
  type PageId,
  type Value,
} from '@koharu/bridge/protocol'

import { isBlockLayer, isTextLayer } from './document'

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

/** Moves a layer under `parent`, or to the page's top level, at `index` counted after it's taken
 * out. */
export function moveLayer(layer: LayerId, parent: LayerId | null, index: number): Promise<Change> {
  return edit('move_layer', [{ op: 'move_layer', layer, parent, index }])
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
 * block's translation's, which is what it draws. */
export function setStyle(layers: Layer[], style: Value): Promise<Change> {
  return edit(
    'set_style',
    layers
      .filter((layer) => isTextLayer(layer) || isBlockLayer(layer))
      .map((layer): Op => {
        const patch: Value = isBlockLayer(layer) ? { translation: { style } } : { style }
        return { op: 'update_layer', layer: layer.id, patch }
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
