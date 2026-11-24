import type { Layer, LayerId, Text } from '@koharu/ipc/protocol'

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

/** The lettering a layer draws: a text's own, or a block's translation, which starts as its source
 * without the words. */
export function letteringOf(layer: Layer): Text | null {
  if (isTextLayer(layer)) return layer
  if (isBlockLayer(layer)) return layer.translation ?? { ...layer.source, content: [] }
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

/** The page's layers among `selected`, in drawing order, leaving out those inside another, which
 * go where it goes. */
export function outermostLayers(layers: Layer[], selected: LayerId[]): LayerId[] {
  return walkLayers(layers)
    .filter(
      ({ layer, ancestors }) =>
        selected.includes(layer.id) && !ancestors.some(({ id }) => selected.includes(id)),
    )
    .map(({ layer }) => layer.id)
}

/** A block's own picture layers: its clean-up, and the mask it shows through. */
export type Part = 'cleanup' | 'mask'

/** Where a drag over a Layers row puts what it carries: before the row's layer in the scene,
 * after it, or into it, as its last children, which draw on top. */
export interface Drop {
  target: LayerId
  zone: 'before' | 'after' | 'inside'
}

/** One `move_layer`: the layer under `parent`, or the page's top level, at `index` counted after
 * it's taken out. */
export interface Move {
  layer: LayerId
  parent: LayerId | null
  index: number
}

/** Whether a layer of the kind `child` may sit in one of the kind `parent`, or on the page for
 * `null`, as the scene allows: a page, a group, and a panel hold any layer but a mask, and a
 * balloon holds blocks and text layers. */
export function mayHold(parent: Layer['kind'] | null, child: Layer['kind']): boolean {
  if (child === 'mask') return false
  if (parent === null || parent === 'group' || parent === 'panel') return true
  return parent === 'balloon' && (child === 'block' || child === 'text')
}

/** The moves putting `dragged`, in their order, at `drop`, one after another; `null` where the
 * scene wouldn't let them go: into themselves, under a locked layer, into a kind that can't hold
 * them, or a locked one moving. Each move's index counts the parent's children as the moves
 * before it left them. */
export function dropMoves(layers: Layer[], dragged: LayerId[], drop: Drop): Move[] | null {
  const target = findLayer(layers, drop.target)
  if (!target) return null
  const chain = [...target.ancestors, target.layer].map((layer) => layer.id)
  if (dragged.some((id) => chain.includes(id))) return null
  const parentPlaced =
    drop.zone === 'inside'
      ? target
      : target.ancestors.length
        ? { layer: target.ancestors.at(-1)!, ancestors: target.ancestors.slice(0, -1) }
        : null
  if (parentPlaced && isLockedLayer(parentPlaced)) return null
  const parent = parentPlaced?.layer ?? null
  for (const id of dragged) {
    const placed = findLayer(layers, id)
    if (!placed || isLockedLayer(placed) || !mayHold(parent?.kind ?? null, placed.layer.kind)) {
      return null
    }
  }
  const before = (parent ? (parent.children ?? []) : layers).map((layer) => layer.id)
  let children = [...before]
  const moves: Move[] = []
  for (const [order, id] of dragged.entries()) {
    children = children.filter((child) => child !== id)
    const index =
      order > 0
        ? children.indexOf(dragged[order - 1]!) + 1
        : drop.zone === 'inside'
          ? children.length
          : children.indexOf(drop.target) + (drop.zone === 'after' ? 1 : 0)
    children.splice(index, 0, id)
    moves.push({ layer: id, parent: parent?.id ?? null, index })
  }
  const unchanged =
    children.length === before.length && children.every((id, at) => id === before[at])
  return unchanged ? [] : moves
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
