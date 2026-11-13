import type { Layer, LayerId, Text } from '@koharu/bridge/protocol'

export type TextLayer = Extract<Layer, { kind: 'text' }>
export type BlockLayer = Extract<Layer, { kind: 'block' }>

/** A layer and the containers above it, outermost first. */
export interface Placed {
  layer: Layer
  ancestors: Layer[]
}

export function isTextLayer(layer: Layer): layer is TextLayer {
  return layer.kind === 'text'
}

export function isBlockLayer(layer: Layer): layer is BlockLayer {
  return layer.kind === 'block'
}

/** Whether a layer holds others: a group, a panel, or a balloon. */
export function isContainer(layer: Layer): boolean {
  return layer.kind === 'group' || layer.kind === 'panel' || layer.kind === 'balloon'
}

/** The lettering a layer draws: a text's own, or a block's translation. */
export function letteringOf(layer: Layer): Text | null {
  if (isTextLayer(layer)) return layer
  if (isBlockLayer(layer)) return layer.translation ?? null
  return null
}

/** Every layer of `layers` and their children, parents first, in drawing order. */
export function walkLayers(layers: Layer[], ancestors: Layer[] = []): Placed[] {
  return layers.flatMap((layer) => [
    { layer, ancestors },
    ...walkLayers(layer.children ?? [], [...ancestors, layer]),
  ])
}

export function findLayer(layers: Layer[], id: LayerId): Placed | undefined {
  return walkLayers(layers).find((placed) => placed.layer.id === id)
}

/** A layer's own children, or none. */
export function layerChildren(layer: Layer): Layer[] {
  return layer.children ?? []
}

/** The selection with each container replaced by the layers inside it. */
export function expandLayerSelection(layers: Layer[], selected: LayerId[]): LayerId[] {
  const result: LayerId[] = []
  const visit = (layer: Layer) => {
    if (isContainer(layer)) {
      for (const child of layerChildren(layer)) visit(child)
    } else if (!result.includes(layer.id)) {
      result.push(layer.id)
    }
  }
  for (const id of selected) {
    const placed = findLayer(layers, id)
    if (placed) visit(placed.layer)
  }
  return result
}

/** Whether a layer shows, and how opaque, with the containers above it. */
export function effectiveLayerVisibility({ layer, ancestors }: Placed) {
  let visible = layer.visible
  let opacity = layer.opacity
  for (const ancestor of ancestors) {
    visible &&= ancestor.visible
    opacity *= ancestor.opacity
  }
  return { visible, opacity }
}

/** Whether a layer or a container above it is locked. */
export function isLockedLayer({ layer, ancestors }: Placed): boolean {
  return layer.locked || ancestors.some((ancestor) => ancestor.locked)
}

/** A rich text's characters. */
export function plainText(text: Text | null | undefined): string {
  return (text?.content ?? []).map((span) => span.insert).join('')
}
