'use client'

import {
  DndContext,
  DragOverlay,
  PointerSensor,
  pointerWithin,
  useDraggable,
  useDroppable,
  useSensor,
  useSensors,
  type DragMoveEvent,
  type Modifier,
} from '@dnd-kit/core'
import { useMutation } from '@tanstack/react-query'
import type { TFunction } from 'i18next'
import {
  Brush,
  createLucideIcon,
  Eye,
  EyeOff,
  Folder,
  Image as ImageIcon,
  Lock,
  LockOpen,
  MessageCircle,
  PanelsTopLeft,
  Shapes,
  Trash2,
  Type,
  X,
} from 'lucide-react'
import { type MouseEvent, useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import {
  dropMoves,
  findLayer,
  isBlockLayer,
  isContainer,
  isLockedLayer,
  isTextLayer,
  plainText,
  walkLayers,
  type BlockLayer,
  type Drop,
  type Part,
  type Placed,
} from '@/lib/document'
import { send, usePage } from '@/lib/queries'
import { deleteLayers, moveLayers, setLocked, setPartVisibility, setVisibility } from '@/lib/scene'
import { useKoharuStore } from '@/lib/store'
import { type Layer, type LayerId } from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'
import { ScrollArea } from '@koharu/ui/components/scroll-area'
import { Slider } from '@koharu/ui/components/slider'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'

/** The page's layers topmost first, as Photoshop's Layers panel lists them, each with its depth:
 * the scene keeps them in reading order and draws the last on top. A block's clean-up and mask
 * are its own, shown on its row, not listed. */
function displayedLayers(layers: Layer[], depth = 0): { placed: Placed; depth: number }[] {
  return layers.toReversed().flatMap((layer) => {
    const placed = { layer, ancestors: [] as Layer[] }
    return [
      { placed, depth },
      ...displayedLayers(layer.children ?? [], depth + 1).map(({ placed: child, depth }) => ({
        placed: { layer: child.layer, ancestors: [layer, ...child.ancestors] },
        depth,
      })),
    ]
  })
}

/** The shown page's layers topmost first, each a row to select, show, lock, fade, or delete, a
 * block's with its clean-up and mask. */
export function LayersPanel() {
  const { t } = useTranslation()
  const page = usePage().data
  const selected = useKoharuStore((state) => state.selectedLayers)
  const selectLayers = useKoharuStore((state) => state.selectLayers)
  const [expandedLayer, setExpandedLayer] = useState<LayerId | null>(
    selected.length === 1 ? (selected[0] ?? null) : null,
  )
  const anchor = useRef<LayerId | null>(null)

  useEffect(() => {
    setExpandedLayer(selected.length === 1 ? (selected[0] ?? null) : null)
  }, [selected])

  const rows = useMemo(() => (page ? displayedLayers(page.layers) : []), [page])
  // A drag of rows: the layers it carries, in the scene's order, and where it would put them now.
  const [drag, setDrag] = useState<{ layers: LayerId[]; drop: Drop | null } | null>(null)
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 4 } }))

  const deletion = useMutation({
    mutationFn: (layer: LayerId) => deleteLayers([layer]),
    onSuccess: (_, layer) => {
      if (selected.includes(layer)) {
        selectLayers(selected.filter((selectedLayer) => selectedLayer !== layer))
      }
      setExpandedLayer((current) => (current === layer ? null : current))
    },
  })

  if (!page) return <EmptyInspector>{t('inspector.selectPage')}</EmptyInspector>

  /** What a drag shows under the pointer: its top row's name, and how many more it carries. */
  const draggedName = (layers: LayerId[]) => {
    const index = rows.findIndex((row) => layers.includes(row.placed.layer.id))
    const first = rows[index]?.placed.layer
    const name = first ? localizedLayerName(first, index, t) : ''
    return layers.length > 1 ? `${name} +${layers.length - 1}` : name
  }

  /** A drag carries every selected layer when it starts on a selected row, else that row's. */
  const carried = (layer: LayerId) =>
    selected.includes(layer)
      ? walkLayers(page.layers)
          .map(({ layer }) => layer.id)
          .filter((id) => selected.includes(id))
      : [layer]

  /** Where the pointer is over the row under it: its top or bottom quarter puts the layers above
   * or below it, and a container's middle into it; another row's halves, above or below. Above is
   * after it in the scene, which draws later layers on top. Only a place the scene allows. */
  const follow = ({ active, over, activatorEvent, delta }: DragMoveEvent) => {
    const layers = drag?.layers ?? carried(String(active.id))
    const target = over ? findLayer(page.layers, String(over.id))?.layer : undefined
    const start = activatorEvent as PointerEvent
    let drop: Drop | null = null
    if (over && target && 'clientY' in start) {
      const at = (start.clientY + delta.y - over.rect.top) / over.rect.height
      const zone: Drop['zone'] = isContainer(target)
        ? at < 0.25
          ? 'after'
          : at > 0.75
            ? 'before'
            : 'inside'
        : at < 0.5
          ? 'after'
          : 'before'
      const candidate = { target: target.id, zone }
      if (dropMoves(page.layers, layers, candidate)) drop = candidate
    }
    setDrag({ layers, drop })
  }

  /** A click on a block's part picks it for the brush, and Shift shows or hides it, as a click
   * and a Shift-click on a layer mask's thumbnail do. */
  const pickPart = (block: BlockLayer, part: Part, toggle: boolean) => {
    const layer = block[part]
    if (!layer) return
    if (toggle) {
      send(setPartVisibility, block.id, part, !layer.visible)
      return
    }
    anchor.current = block.id
    selectLayers([layer.id])
  }

  // Modifier semantics mirror the page rail: ctrl/meta toggles, shift selects a display range.
  const selectLayer = (layer: LayerId, additive: boolean, range: boolean) => {
    if (!additive && !range) {
      anchor.current = layer
      if (selected.length === 1 && selected[0] === layer) {
        setExpandedLayer((current) => (current === layer ? null : layer))
        return
      }
      selectLayers([layer])
      setExpandedLayer(layer)
      return
    }
    const order = rows.map((row) => row.placed.layer.id)
    const anchorIndex = anchor.current ? order.indexOf(anchor.current) : -1
    if (range && anchorIndex >= 0) {
      const targetIndex = order.indexOf(layer)
      const span = order.slice(
        Math.min(anchorIndex, targetIndex),
        Math.max(anchorIndex, targetIndex) + 1,
      )
      selectLayers(additive ? [...selected, ...span] : span)
      return
    }
    anchor.current = layer
    selectLayers(
      selected.includes(layer)
        ? selected.filter((selectedLayer) => selectedLayer !== layer)
        : [...selected, layer],
    )
  }

  return (
    <div className='flex min-h-0 flex-1 flex-col'>
      <DndContext
        sensors={sensors}
        collisionDetection={pointerWithin}
        onDragStart={({ active }) => setDrag({ layers: carried(String(active.id)), drop: null })}
        onDragMove={follow}
        onDragCancel={() => setDrag(null)}
        onDragEnd={() => {
          const moves = drag?.drop ? dropMoves(page.layers, drag.layers, drag.drop) : null
          setDrag(null)
          if (moves?.length) send(moveLayers, moves)
        }}
      >
        <ScrollArea className='min-h-0 flex-1'>
          <div className='py-0.5'>
            {rows.map(({ placed, depth }, index) => {
              const { layer } = placed
              return (
                <LayerRow
                  key={layer.id}
                  layer={layer}
                  index={index}
                  depth={depth}
                  selected={selected.includes(layer.id)}
                  picked={
                    isBlockLayer(layer)
                      ? (['cleanup', 'mask'] as const).find((part) =>
                          selected.includes(layer[part]?.id ?? ''),
                        )
                      : undefined
                  }
                  onPick={(part, toggle) => isBlockLayer(layer) && pickPart(layer, part, toggle)}
                  expanded={expandedLayer === layer.id}
                  onSelect={(event) =>
                    selectLayer(layer.id, event.ctrlKey || event.metaKey, event.shiftKey)
                  }
                  onToggle={() => send(setVisibility, [layer.id], !layer.visible, null)}
                  onLock={() => send(setLocked, [layer.id], !layer.locked)}
                  movable={!isLockedLayer(placed)}
                  carried={drag?.layers.includes(layer.id) ?? false}
                  drop={drag?.drop?.target === layer.id ? drag.drop.zone : undefined}
                  onDelete={() => deletion.mutate(layer.id)}
                />
              )
            })}
            {rows.length === 0 && <EmptyInspector>{t('layers.empty')}</EmptyInspector>}
          </div>
        </ScrollArea>
        <DragOverlay dropAnimation={null} modifiers={[besidePointer]}>
          {drag && (
            <div className='w-fit max-w-56 truncate rounded-md border border-border bg-popover px-2 py-1 text-[11px] font-medium shadow-sm'>
              {draggedName(drag.layers)}
            </div>
          )}
        </DragOverlay>
      </DndContext>
    </div>
  )
}

function LayerRow({
  layer,
  index,
  depth,
  selected,
  picked,
  expanded,
  onSelect,
  onPick,
  onToggle,
  onLock,
  movable,
  carried,
  drop,
  onDelete,
}: {
  layer: Layer
  index: number
  depth: number
  selected: boolean
  /** The block's part selected for the brush, if one is. */
  picked: Part | undefined
  expanded: boolean
  onSelect: (event: MouseEvent<HTMLButtonElement>) => void
  onPick: (part: Part, toggle: boolean) => void
  onToggle: () => void
  onLock: () => void
  /** Whether it can be dragged: not locked, nor in a locked layer. */
  movable: boolean
  /** Whether the drag in progress carries it. */
  carried: boolean
  /** Where the drag in progress would put its layers relative to this row's layer in the scene:
   * after it shows above the row, before it below. */
  drop: Drop['zone'] | undefined
  onDelete: () => void
}) {
  const { t } = useTranslation()
  const name = localizedLayerName(layer, index, t)
  const detail = t(`layers.kinds.${layer.kind}`)
  const Icon = layerIcon(layer)
  const block = isBlockLayer(layer) ? layer : null
  const handle = useDraggable({ id: layer.id, disabled: !movable })
  const target = useDroppable({ id: layer.id })
  return (
    <div className='group min-w-0 px-1 py-px' style={{ paddingLeft: `${depth * 10 + 4}px` }}>
      <div
        data-selected={selected || picked !== undefined}
        data-expanded={expanded}
        data-carried={carried}
        data-drop={drop === 'inside'}
        className='min-w-0 overflow-hidden rounded-lg transition-colors duration-150 data-[carried=true]:opacity-50 data-[drop=true]:bg-primary/10 data-[drop=true]:ring-1 data-[drop=true]:ring-primary data-[selected=true]:bg-accent motion-reduce:transition-none'
      >
        <div
          ref={(element) => {
            handle.setNodeRef(element)
            target.setNodeRef(element)
          }}
          {...handle.listeners}
          className='relative flex min-w-0 items-center gap-0.5'
        >
          {(drop === 'before' || drop === 'after') && (
            <span
              aria-hidden='true'
              className={`pointer-events-none absolute inset-x-1 z-10 h-0.5 rounded-full bg-primary ${drop === 'after' ? '-top-px' : '-bottom-px'}`}
            />
          )}
          <button
            type='button'
            aria-label={t('layers.edit', { name })}
            aria-expanded={expanded}
            className='flex min-w-0 flex-1 items-center gap-1.5 rounded-lg px-1.5 py-1 text-left hover:bg-foreground/[0.05] focus-visible:ring-2 focus-visible:ring-ring/25'
            onClick={onSelect}
          >
            <Icon className='size-3.5 shrink-0 text-muted-foreground' />
            <span className='min-w-0 flex-1'>
              <span className='block truncate text-[11px] font-medium'>{name}</span>
              <span className='block truncate text-[9px] leading-3 text-muted-foreground'>
                {detail}
              </span>
            </span>
          </button>
          {block &&
            (['cleanup', 'mask'] as const).map((part) => (
              <PartButton
                key={part}
                block={block}
                part={part}
                name={name}
                picked={picked === part}
                onPick={(toggle) => onPick(part, toggle)}
              />
            ))}
          <Tooltip>
            <TooltipTrigger
              render={
                <button
                  type='button'
                  aria-label={
                    layer.locked ? t('layers.unlock', { name }) : t('layers.lock', { name })
                  }
                  aria-pressed={layer.locked}
                  className='grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground hover:bg-foreground/[0.06] hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/25'
                  onClick={onLock}
                />
              }
            >
              {layer.locked ? (
                <Lock className='size-3.5' />
              ) : (
                <LockOpen className='size-3.5 opacity-40' />
              )}
            </TooltipTrigger>
            <TooltipContent side='left'>
              {layer.locked ? t('layers.locked') : t('layers.unlocked')}
            </TooltipContent>
          </Tooltip>
          <button
            type='button'
            aria-label={layer.visible ? t('layers.hide', { name }) : t('layers.show', { name })}
            className='grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground hover:bg-foreground/[0.06] hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/25'
            onClick={onToggle}
          >
            {layer.visible ? <Eye className='size-3.5' /> : <EyeOff className='size-3.5' />}
          </button>
        </div>
        {expanded && (
          <div className='animate-in duration-150 fade-in slide-in-from-top-1 motion-reduce:animate-none'>
            <LayerEditor layer={layer} onDelete={layer.locked ? undefined : onDelete} />
          </div>
        )}
      </div>
    </div>
  )
}

function LayerEditor({ layer, onDelete }: { layer: Layer; onDelete?: () => void }) {
  const { t } = useTranslation()
  const name = localizedLayerName(layer, 0, t)
  const [opacity, setOpacity] = useState(layer.opacity * 100)

  useEffect(() => {
    setOpacity(layer.opacity * 100)
  }, [layer.id, layer.opacity])

  const fade = useMutation({
    mutationFn: (next: number) => setVisibility([layer.id], null, next / 100),
    onError: () => setOpacity(layer.opacity * 100),
  })

  return (
    <div className='grid min-w-0 gap-1.5 px-1.5 pt-0.5 pb-1.5'>
      <div className='flex min-w-0 items-center gap-1.5'>
        <span className='shrink-0 text-[8px] font-medium text-muted-foreground uppercase'>
          {t('inspector.opacity')}
        </span>
        <div className='flex min-w-0 flex-1 items-center gap-1.5'>
          <Slider
            aria-label={t('layers.opacityLabel', { name })}
            min={0}
            max={100}
            step={1}
            value={opacity}
            disabled={layer.locked}
            className='[&_[data-slot=slider-thumb]]:size-2'
            onValueChange={setOpacity}
            onValueCommitted={(next) => fade.mutate(next)}
          />
          <span className='w-7 shrink-0 text-right text-[8px] text-muted-foreground tabular-nums'>
            {Math.round(opacity)}%
          </span>
        </div>
        {onDelete && (
          <Tooltip>
            <TooltipTrigger
              render={
                <Button
                  type='button'
                  variant='ghost'
                  size='icon-xs'
                  aria-label={t('layers.delete', { name })}
                  className='size-5 rounded-md text-muted-foreground hover:text-foreground'
                  onClick={onDelete}
                />
              }
            >
              <Trash2 className='size-3' />
            </TooltipTrigger>
            <TooltipContent side='left'>{t('layers.delete', { name })}</TooltipContent>
          </Tooltip>
        )}
      </div>
    </div>
  )
}

// Drawn on lucide's grid, in its two-unit strokes, so they sit with its icons.
const turned = 'rotate(-45 12 12)'

/** A bandage on the diagonal, as Photoshop's healing tools draw theirs. */
const Healing = createLucideIcon('healing', [
  ['rect', { x: '3', y: '7', width: '18', height: '10', rx: '2', transform: turned, key: 'strip' }],
  ['path', { d: 'M7 7v10M17 7v10', transform: turned, key: 'pad' }],
  [
    'path',
    {
      d: 'M10.5 10.5h.01M13.5 10.5h.01M10.5 13.5h.01M13.5 13.5h.01',
      transform: turned,
      key: 'dots',
    },
  ],
])

/** Photoshop's layer mask symbol, a frame round a filled disc. The disc takes no stroke, which
 * would grow it by a unit all round and outweigh the row's other icons. */
const LayerMask = createLucideIcon('layer-mask', [
  ['rect', { x: '3', y: '3', width: '18', height: '18', rx: '2', key: 'frame' }],
  ['circle', { cx: '12', cy: '12', r: '4', fill: 'currentColor', stroke: 'none', key: 'disc' }],
])

/** A block's clean-up or mask on its row: the clean-up as a bandage, as Photoshop's healing tools
 * draw theirs, since it heals the art under the source text, and the mask as a layer mask. Ringed
 * while the brush paints it and crossed out in red while hidden, as a layer mask's thumbnail is. */
function PartButton({
  block,
  part,
  name,
  picked,
  onPick,
}: {
  block: BlockLayer
  part: Part
  name: string
  picked: boolean
  onPick: (toggle: boolean) => void
}) {
  const { t } = useTranslation()
  const layer = block[part]
  if (!layer) return null
  const label = t(`layers.parts.${part}`)
  const Icon = part === 'cleanup' ? Healing : LayerMask
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <button
            type='button'
            aria-label={t('layers.parts.pick', { part: label, name })}
            aria-pressed={picked}
            data-part={part}
            data-picked={picked}
            className='relative grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground hover:bg-foreground/[0.06] hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/25 data-[picked=true]:bg-primary/15 data-[picked=true]:text-primary data-[picked=true]:ring-1 data-[picked=true]:ring-primary'
            onClick={(event) => onPick(event.shiftKey)}
          />
        }
      >
        <Icon className={layer.visible ? 'size-3.5' : 'size-3.5 opacity-40'} />
        {!layer.visible && (
          <X className='absolute inset-0 m-auto size-4 text-destructive' strokeWidth={2.5} />
        )}
      </TooltipTrigger>
      <TooltipContent side='left'>
        {label}
        <span className='ml-2 opacity-60'>{t('layers.parts.toggle')}</span>
      </TooltipContent>
    </Tooltip>
  )
}

/** A drag's chip below and right of the pointer, as a system drag image sits, rather than over
 * its row's place, so the row under the pointer stays readable. */
const besidePointer: Modifier = ({ transform, activatorEvent, activeNodeRect }) =>
  activatorEvent instanceof PointerEvent && activeNodeRect
    ? {
        ...transform,
        x: transform.x + activatorEvent.clientX - activeNodeRect.left + 12,
        y: transform.y + activatorEvent.clientY - activeNodeRect.top + 12,
      }
    : transform

function EmptyInspector({ children }: { children: React.ReactNode }) {
  return (
    <div className='px-4 py-8 text-center text-[10px] leading-4 text-muted-foreground'>
      {children}
    </div>
  )
}

function layerIcon(layer: Layer): typeof Type {
  switch (layer.kind) {
    case 'group':
      return Folder
    case 'raster':
      return Brush
    case 'text':
    case 'block':
      return Type
    case 'panel':
      return PanelsTopLeft
    case 'balloon':
      return MessageCircle
    case 'shape':
      return Shapes
    case 'mask':
      return LayerMask
    default:
      return ImageIcon
  }
}

function localizedLayerName(layer: Layer, index: number, t: TFunction): string {
  if (layer.name) return layer.name
  if (isTextLayer(layer))
    return plainText(layer).trim() || t('layers.textName', { index: index + 1 })
  if (isBlockLayer(layer)) {
    return (
      plainText(layer.translation).trim() ||
      plainText(layer.source).trim() ||
      t('layers.textName', { index: index + 1 })
    )
  }
  return t(`layers.kinds.${layer.kind}`)
}
