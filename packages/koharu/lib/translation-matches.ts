import type { Page } from '@koharu/ipc/protocol'

import { isBlockLayer, isLockedLayer, plainText, walkLayers, type BlockLayer } from './document'

function uniqueSources(page: Page) {
  const entries = new Map<string, { layer: BlockLayer; locked: boolean } | null>()
  for (const placed of walkLayers(page.layers)) {
    if (!isBlockLayer(placed.layer)) continue
    const key = plainText(placed.layer.source).trim().normalize('NFC')
    if (key)
      entries.set(
        key,
        entries.has(key) ? null : { layer: placed.layer, locked: isLockedLayer(placed) },
      )
  }
  return entries
}

export function translationMatches(source: Page, target: Page) {
  const matches: { target: string; ocr: string; text: string }[] = []
  const sources = uniqueSources(source)
  for (const [ocr, dest] of uniqueSources(target)) {
    const from = sources.get(ocr)?.layer
    if (!from || !dest) continue
    if (dest.locked || plainText(dest.layer.translation)) continue
    const text = plainText(from.translation)
    if (text.trim()) matches.push({ target: dest.layer.id, ocr, text })
  }
  return matches
}
