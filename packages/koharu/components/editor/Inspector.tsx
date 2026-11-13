'use client'

import type { TFunction } from 'i18next'
import {
  AlignCenter,
  AlignLeft,
  AlignRight,
  ArrowDown,
  ArrowUp,
  Brush,
  ChevronDown,
  Eye,
  EyeOff,
  Folder,
  Image as ImageIcon,
  Layers3,
  Lock,
  LockOpen,
  Minus,
  MessageCircle,
  PanelsTopLeft,
  Plus,
  Shapes,
  SquareDashed,
  Trash2,
  Type,
} from 'lucide-react'
import { type MouseEvent, useEffect, useId, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { ColorWell } from '@/components/controls/ColorWell'
import { CommitTextarea } from '@/components/controls/CommitTextarea'
import { FontPicker } from '@/components/controls/FontPicker'
import { call, dispatch } from '@/lib/backend'
import {
  expandLayerSelection,
  findLayer,
  isBlockLayer,
  isTextLayer,
  letteringOf,
  plainText,
  walkLayers,
  type Placed,
} from '@/lib/document'
import { useFonts, usePage } from '@/lib/queries'
import { hexToRgba, rgbaToHex, useKoharuStore } from '@/lib/store'
import {
  commands,
  type Align,
  type Direction,
  type FontFamily,
  type Layer,
  type LayerId,
  type Text,
  type TextStyle,
  type Value,
} from '@koharu/bridge/protocol'
import { Button } from '@koharu/ui/components/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from '@koharu/ui/components/dropdown-menu'
import {
  NumberField,
  NumberFieldDecrement,
  NumberFieldGroup,
  NumberFieldIncrement,
  NumberFieldInput,
} from '@koharu/ui/components/number-field'
import { ScrollArea } from '@koharu/ui/components/scroll-area'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@koharu/ui/components/select'
import { Slider } from '@koharu/ui/components/slider'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'

/** The renderer's own default when `[renderer]` names no font. */
const FALLBACK_FONT = 'CCWildWords'
const BLACK = '#000000'
const WHITE = '#FFFFFF'

export function Inspector() {
  const { t } = useTranslation()
  return (
    <aside className='flex h-full min-h-0 flex-col bg-[var(--surface-panel)]'>
      <div className='flex h-8 shrink-0 items-center gap-1.5 border-b border-border/80 px-2'>
        <Type className='size-3 text-primary' />
        <h2 className='text-[10px] font-semibold'>{t('inspector.type')}</h2>
      </div>

      <section className='h-48 min-w-0 shrink-0 overflow-hidden border-b'>
        <TypeInspector />
      </section>

      <section className='flex min-h-0 flex-1 flex-col'>
        <LayersInspector />
      </section>
    </aside>
  )
}

/** A text's or a block's translation's lettering: the style it draws with, its direction, and
 * its box's fit. A change goes to every text and block selected. */
function TypeInspector() {
  const { t } = useTranslation()
  const borderWidthId = useId()
  const page = usePage().data
  const selectedIds = useKoharuStore((state) => state.selectedLayers)
  const defaultFont = useKoharuStore(
    (state) => state.preferences?.renderer.font_families?.[0] ?? FALLBACK_FONT,
  )
  const availableFonts = useFonts().data
  const targets = useMemo(() => {
    if (!page) return []
    const leaves = expandLayerSelection(page.layers, selectedIds)
    return walkLayers(page.layers)
      .map(({ layer }) => layer)
      .filter((layer) => leaves.includes(layer.id) && (isTextLayer(layer) || isBlockLayer(layer)))
  }, [page, selectedIds])
  const current = targets[0]
  const lettering: Text | null = current ? letteringOf(current) : null
  const [draft, setDraft] = useState<{ layer: LayerId; style: TextStyle } | null>(null)
  const sequence = useRef(0)

  useEffect(() => setDraft(null), [current?.id])

  const style = current && draft?.layer === current.id ? draft.style : (lettering?.style ?? {})
  const chars = style.chars ?? {}
  const disabled = !current
  const frame = lettering?.frame ?? 'point'
  const box = typeof frame === 'object' && 'box' in frame ? frame.box : null
  const autoFit = box?.fit === 'best'

  /** Merges `patch` into each target's style, shown at once on the first. */
  const apply = (patch: TextStyle) => {
    if (!current) return
    setDraft({
      layer: current.id,
      style: { ...style, ...patch, chars: { ...chars, ...patch.chars } },
    })
    const request = ++sequence.current
    void call(
      commands.setStyle,
      targets.map((layer) => layer.id),
      patch,
    )
      .catch(() => undefined)
      .finally(() => {
        if (sequence.current === request) setDraft(null)
      })
  }

  /** Merges `patch` into each target's lettering: a text's own values, a block's translation's. */
  const update = (patch: { [key in string]: Value }) => {
    for (const layer of targets) {
      dispatch(commands.updateLayer, layer.id, isBlockLayer(layer) ? { translation: patch } : patch)
    }
  }

  const families = useMemo(() => {
    const available = availableFonts ?? []
    return available.some((family) => sameFont(family.name, defaultFont))
      ? available
      : [...available, placeholderFamily(defaultFont)]
  }, [availableFonts, defaultFont])
  const family = chars.family ?? defaultFont
  const selectedFamily = families.find((candidate) => sameFont(candidate.name, family))
  const italics = hasItalic(selectedFamily)
  const italic = Boolean(chars.italic) && italics
  const weight = chars.weight ?? 400
  const weights = usableFontWeights(selectedFamily, italic)
  const size = Math.round((chars.size ?? 24) * 100) / 100
  const strokeWidth = chars.stroke_width ?? 0
  const stroke = chars.stroke ?? null
  const strokeEnabled = stroke !== null && stroke[3] > 0 && strokeWidth > 0
  const direction: Direction = lettering?.direction ?? 'horizontal'
  const align: Align = style.align ?? (direction === 'vertical' ? 'start' : 'center')

  return (
    <div className='min-w-0 p-2' data-testid='type-inspector' aria-disabled={disabled}>
      <div className='grid min-w-0 gap-1.5'>
        <div className='grid min-w-0 grid-cols-[minmax(0,1fr)_2.5rem] gap-1.5'>
          <InspectorField label={t('inspector.font')}>
            <FontPicker
              value={family}
              families={families}
              disabled={disabled}
              size='sm'
              onChange={(next) => {
                const nextFamily = families.find((candidate) => sameFont(candidate.name, next))
                const nextItalic = italic && hasItalic(nextFamily)
                apply({
                  chars: {
                    family: next,
                    italic: nextItalic,
                    weight: nearestFontWeight(usableFontWeights(nextFamily, nextItalic), weight),
                  },
                })
              }}
            />
          </InspectorField>
          <InspectorField label={t('inspector.color')}>
            <ColorWell
              label={t('inspector.textColor')}
              size='sm'
              disabled={disabled}
              value={chars.fill ? rgbaToHex(chars.fill) : BLACK}
              onChange={(color) => color && apply({ chars: { fill: hexToRgba(color) } })}
            />
          </InspectorField>
        </div>

        <div className='grid min-w-0 grid-cols-[minmax(0,1fr)_4.25rem_4.75rem] gap-1.5'>
          <InspectorField label={t('inspector.size')}>
            <FontSizeField
              disabled={disabled}
              value={size}
              autoFit={autoFit}
              canAutoFit={box !== null}
              onChange={(next) => {
                apply({ chars: { size: next } })
                // A set size is the most the text takes, as the box's best fit would grow it.
                if (autoFit) update({ frame: { box: { fit: 'shrink' } } })
              }}
              onAutoFit={() => update({ frame: { box: { fit: 'best' } } })}
            />
          </InspectorField>
          <InspectorField label={t('inspector.weight')}>
            <Select
              disabled={disabled}
              value={String(weight)}
              onValueChange={(next) => next && apply({ chars: { weight: Number(next) } })}
            >
              <SelectTrigger
                size='sm'
                aria-label={t('inspector.fontWeight')}
                className='w-full min-w-0'
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {weights.map((value) => (
                  <SelectItem key={value} value={String(value)}>
                    {value}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </InspectorField>
          <InspectorField label={t('inspector.style')}>
            <Select
              disabled={disabled || !italics}
              value={italic ? 'italic' : 'normal'}
              onValueChange={(next) => {
                const nextItalic = next === 'italic'
                apply({
                  chars: {
                    italic: nextItalic,
                    weight: nearestFontWeight(usableFontWeights(selectedFamily, nextItalic), weight),
                  },
                })
              }}
            >
              <SelectTrigger
                size='sm'
                aria-label={t('inspector.fontStyle')}
                className='w-full min-w-0'
              >
                <SelectValue>{t(`inspector.fontStyles.${italic ? 'italic' : 'normal'}`)}</SelectValue>
              </SelectTrigger>
              <SelectContent align='end'>
                <SelectItem value='normal'>{t('inspector.fontStyles.normal')}</SelectItem>
                {italics && <SelectItem value='italic'>{t('inspector.fontStyles.italic')}</SelectItem>}
              </SelectContent>
            </Select>
          </InspectorField>
        </div>

        <div className='grid min-w-0 grid-cols-[minmax(0,1fr)_minmax(6.5rem,1fr)] gap-1.5'>
          <InspectorField label={t('inspector.alignment')}>
            <div className='grid h-6 grid-cols-3 rounded-md border border-input bg-background p-px'>
              {(
                [
                  [
                    'start',
                    AlignLeft,
                    direction === 'vertical' ? t('inspector.alignTop') : t('inspector.alignLeft'),
                  ],
                  [
                    'center',
                    AlignCenter,
                    direction === 'vertical'
                      ? t('inspector.alignMiddle')
                      : t('inspector.alignCenter'),
                  ],
                  [
                    'end',
                    AlignRight,
                    direction === 'vertical'
                      ? t('inspector.alignBottom')
                      : t('inspector.alignRight'),
                  ],
                ] as const
              ).map(([value, Icon, label]) => (
                <button
                  key={value}
                  type='button'
                  aria-label={label}
                  aria-pressed={align === value}
                  disabled={disabled}
                  data-active={align === value}
                  className='grid place-items-center rounded-[4px] text-muted-foreground hover:text-foreground disabled:cursor-not-allowed disabled:opacity-40 data-[active=true]:bg-primary data-[active=true]:text-primary-foreground data-[active=true]:hover:bg-primary/90'
                  onClick={() => apply({ align: value })}
                >
                  <Icon className='size-3' />
                </button>
              ))}
            </div>
          </InspectorField>
          <InspectorField label={t('inspector.direction')}>
            <Select
              disabled={disabled}
              value={direction}
              items={{
                horizontal: t('inspector.horizontal'),
                vertical: t('inspector.vertical'),
              }}
              onValueChange={(next) => next && update({ direction: next })}
            >
              <SelectTrigger size='sm' aria-label={t('inspector.textDirection')} className='w-full'>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value='horizontal'>{t('inspector.horizontal')}</SelectItem>
                <SelectItem value='vertical'>{t('inspector.vertical')}</SelectItem>
              </SelectContent>
            </Select>
          </InspectorField>
        </div>

        <div className='grid min-w-0 grid-cols-[2.75rem_minmax(5.5rem,1fr)] items-end gap-1.5'>
          <InspectorField label={t('inspector.border')}>
            <ColorWell
              label={t('inspector.borderColor')}
              size='sm'
              disabled={disabled}
              allowTransparent
              value={strokeEnabled && stroke ? rgbaToHex(stroke) : null}
              onChange={(color) =>
                apply({
                  chars:
                    color === null
                      ? { stroke: null }
                      : {
                          stroke: hexToRgba(color),
                          stroke_width: strokeWidth > 0 ? strokeWidth : 1.5,
                        },
                })
              }
            />
          </InspectorField>
          <InspectorField label={t('inspector.width')}>
            <NumberField
              id={borderWidthId}
              name='border-width'
              className='min-w-0'
              disabled={disabled}
              value={strokeWidth > 0 ? strokeWidth : 1.5}
              min={0.5}
              max={32}
              step={0.5}
              onValueChange={(next) => {
                if (next !== null && next >= 0.5 && next <= 32) {
                  apply({
                    chars: { stroke_width: next, stroke: stroke ?? hexToRgba(WHITE) },
                  })
                }
              }}
            >
              <NumberFieldGroup>
                <NumberFieldDecrement aria-label={t('inspector.decreaseBorderWidth')}>
                  <Minus />
                </NumberFieldDecrement>
                <NumberFieldInput aria-label={t('inspector.borderWidth')} />
                <NumberFieldIncrement aria-label={t('inspector.increaseBorderWidth')}>
                  <Plus />
                </NumberFieldIncrement>
              </NumberFieldGroup>
            </NumberField>
          </InspectorField>
        </div>
      </div>
    </div>
  )
}

/** A family the font list hasn't loaded, so the default still shows. */
function placeholderFamily(name: string): FontFamily {
  return {
    name,
    metadata: {
      primary_script: null,
      scripts: [],
      languages: [],
      category: null,
      classifications: [],
      use_cases: [],
    },
    sources: ['bundled'],
    faces: [{ post_script_name: name, weight: 400, weight_range: null, style: 'normal' }],
  }
}

function hasItalic(family: FontFamily | undefined): boolean {
  return Boolean(family?.faces.some((face) => face.style !== 'normal'))
}

function usableFontWeights(family: FontFamily | undefined, italic: boolean): number[] {
  if (!family) return [400]
  const styled = family.faces.filter((face) => (face.style !== 'normal') === italic)
  const faces = styled.length ? styled : family.faces
  const weights = new Set(faces.map((face) => face.weight))
  for (const face of faces) {
    if (!face.weight_range) continue
    const { start, end } = face.weight_range
    weights.add(start)
    weights.add(end)
    for (let weight = 100; weight <= 900; weight += 100) {
      if (weight >= start && weight <= end) weights.add(weight)
    }
  }
  return [...weights].sort((left, right) => left - right)
}

function nearestFontWeight(weights: number[], target: number): number {
  return weights.reduce(
    (nearest, weight) => (Math.abs(weight - target) < Math.abs(nearest - target) ? weight : nearest),
    weights[0] ?? target,
  )
}

function sameFont(left: string, right: string): boolean {
  const normalize = (value: string) =>
    value.normalize('NFKC').trim().replace(/\s+/g, ' ').toLowerCase()
  return normalize(left) === normalize(right)
}

/** The page's layers, frontmost first as Photoshop's Layers panel lists them, each with its
 * depth. A block's mask and clean-up are its own and aren't listed. */
function displayedLayers(layers: Layer[], depth = 0): { placed: Placed; depth: number }[] {
  return [...layers].reverse().flatMap((layer) => {
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

function LayersInspector() {
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

  if (!page) return <EmptyInspector>{t('inspector.selectPage')}</EmptyInspector>

  /** The layer's siblings in drawing order, and its parent. */
  const siblingsOf = (layer: Layer) => {
    const parent = findLayer(page.layers, layer.id)?.ancestors.at(-1) ?? null
    return { parent, siblings: parent ? (parent.children ?? []) : page.layers }
  }

  /** Moves a layer toward the front (`1`) or the back (`-1`) among its siblings. */
  const move = (layer: Layer, toward: 1 | -1) => {
    const { parent, siblings } = siblingsOf(layer)
    const index = siblings.findIndex((sibling) => sibling.id === layer.id)
    const target = index + toward
    if (index < 0 || target < 0 || target >= siblings.length) return
    // `move_layer` counts the index after the layer is taken out, which is `target` either way.
    dispatch(commands.moveLayer, layer.id, parent?.id ?? null, target)
  }

  const deleteLayer = (layer: LayerId) =>
    void call(commands.deleteLayers, [layer])
      .then(() => {
        if (selected.includes(layer)) {
          selectLayers(selected.filter((selectedLayer) => selectedLayer !== layer))
        }
        setExpandedLayer((current) => (current === layer ? null : current))
      })
      .catch(() => undefined)

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
      <header className='flex h-8 shrink-0 items-center gap-1.5 border-b border-border/80 px-2'>
        <Layers3 className='size-3 text-primary' />
        <h2 className='text-[10px] font-semibold'>{t('layers.title')}</h2>
        <span className='text-[9px] text-muted-foreground tabular-nums'>{rows.length}</span>
      </header>

      <ScrollArea className='min-h-0 flex-1'>
        <div className='py-0.5'>
          {rows.map(({ placed, depth }, index) => {
            const { layer } = placed
            const { siblings } = siblingsOf(layer)
            const position = siblings.findIndex((sibling) => sibling.id === layer.id)
            return (
              <LayerRow
                key={layer.id}
                layer={layer}
                index={index}
                depth={depth}
                selected={selected.includes(layer.id)}
                expanded={expandedLayer === layer.id}
                onSelect={(event) =>
                  selectLayer(layer.id, event.ctrlKey || event.metaKey, event.shiftKey)
                }
                onToggle={() => dispatch(commands.setVisibility, [layer.id], !layer.visible, null)}
                onLock={() => dispatch(commands.setLocked, [layer.id], !layer.locked)}
                onMove={(toward) => move(layer, toward)}
                canMoveUp={!layer.locked && position < siblings.length - 1}
                canMoveDown={!layer.locked && position > 0}
                onDelete={() => deleteLayer(layer.id)}
              />
            )
          })}
          {rows.length === 0 && <EmptyInspector>{t('layers.empty')}</EmptyInspector>}
        </div>
      </ScrollArea>
    </div>
  )
}

function LayerRow({
  layer,
  index,
  depth,
  selected,
  expanded,
  onSelect,
  onToggle,
  onLock,
  onMove,
  canMoveUp,
  canMoveDown,
  onDelete,
}: {
  layer: Layer
  index: number
  depth: number
  selected: boolean
  expanded: boolean
  onSelect: (event: MouseEvent<HTMLButtonElement>) => void
  onToggle: () => void
  onLock: () => void
  onMove: (toward: 1 | -1) => void
  canMoveUp: boolean
  canMoveDown: boolean
  onDelete: () => void
}) {
  const { t } = useTranslation()
  const name = localizedLayerName(layer, index, t)
  const detail = t(`layers.kinds.${layer.kind}`)
  const Icon = layerIcon(layer)
  return (
    <div className='group min-w-0 px-1 py-px' style={{ paddingLeft: `${depth * 10 + 4}px` }}>
      <div
        data-selected={selected}
        data-expanded={expanded}
        className='min-w-0 overflow-hidden rounded-lg transition-colors duration-150 data-[selected=true]:bg-accent motion-reduce:transition-none'
      >
        <div className='relative flex min-w-0 items-center gap-0.5'>
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
          <div
            className={`pointer-events-none absolute top-1/2 z-10 flex -translate-y-1/2 rounded-md bg-background/80 p-0.5 opacity-0 shadow-sm ring-1 ring-border/40 backdrop-blur-md transition-opacity duration-150 group-hover:pointer-events-auto group-hover:opacity-100 focus-within:pointer-events-auto focus-within:opacity-100 motion-reduce:transition-none ${expanded ? 'right-[3.25rem]' : 'right-[4.75rem]'}`}
          >
            <button
              type='button'
              aria-label={t('layers.moveUp', { name })}
              disabled={!canMoveUp}
              className='grid size-5 place-items-center rounded-sm text-muted-foreground hover:bg-foreground/[0.07] hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/25 disabled:pointer-events-none disabled:opacity-30'
              onClick={() => onMove(1)}
            >
              <ArrowUp className='size-3' />
            </button>
            <button
              type='button'
              aria-label={t('layers.moveDown', { name })}
              disabled={!canMoveDown}
              className='grid size-5 place-items-center rounded-sm text-muted-foreground hover:bg-foreground/[0.07] hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/25 disabled:pointer-events-none disabled:opacity-30'
              onClick={() => onMove(-1)}
            >
              <ArrowDown className='size-3' />
            </button>
          </div>
          {!expanded && (
            <span className='w-7 shrink-0 text-right text-[9px] text-muted-foreground tabular-nums'>
              {Math.round(layer.opacity * 100)}%
            </span>
          )}
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
              {layer.locked ? <Lock className='size-3.5' /> : <LockOpen className='size-3.5 opacity-40' />}
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

  const commitOpacity = (next: number) =>
    void call(commands.setVisibility, [layer.id], null, next / 100).catch(() =>
      setOpacity(layer.opacity * 100),
    )

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
            onValueCommitted={commitOpacity}
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
      {isBlockLayer(layer) && (
        <>
          <InspectorField label={t('inspector.source')}>
            <CommitTextarea
              data-testid={`edit-source-${layer.id}`}
              aria-label={t('layers.sourceLabel', { name })}
              wrap='soft'
              disabled={layer.locked}
              className='max-h-14 min-h-8 w-full max-w-full min-w-0 resize-y overflow-y-auto rounded-md bg-background px-1.5 py-1 text-[12px] leading-4 md:text-[12px]'
              value={plainText(layer.source)}
              onCommit={(text) => dispatch(commands.setSourceText, layer.id, text)}
            />
          </InspectorField>
          <InspectorField label={t('inspector.translation')}>
            <CommitTextarea
              data-testid={`edit-translation-${layer.id}`}
              aria-label={t('layers.translationLabel', { name })}
              wrap='soft'
              disabled={layer.locked}
              className='max-h-16 min-h-9 w-full max-w-full min-w-0 resize-y overflow-y-auto rounded-md border-primary/25 bg-background px-1.5 py-1 text-[12px] leading-4 md:text-[12px]'
              value={plainText(layer.translation)}
              onCommit={(text) => dispatch(commands.setText, layer.id, text)}
            />
          </InspectorField>
        </>
      )}
      {isTextLayer(layer) && (
        <InspectorField label={t('inspector.text')}>
          <CommitTextarea
            data-testid={`edit-text-${layer.id}`}
            aria-label={t('layers.textLabel', { name })}
            wrap='soft'
            disabled={layer.locked}
            className='max-h-16 min-h-9 w-full max-w-full min-w-0 resize-y overflow-y-auto rounded-md bg-background px-1.5 py-1 text-[12px] leading-4 md:text-[12px]'
            value={plainText(layer)}
            onCommit={(text) => dispatch(commands.setText, layer.id, text)}
          />
        </InspectorField>
      )}
    </div>
  )
}

function InspectorField({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className='grid min-w-0 gap-0.5'>
      <span className='text-[8px] font-medium tracking-[0.06em] text-muted-foreground uppercase'>
        {label}
      </span>
      {children}
    </div>
  )
}

const fontSizePresets = [8, 9, 10, 12, 14, 16, 18, 20, 24, 28, 32, 36, 48, 64, 72, 96]

function FontSizeField({
  value,
  autoFit,
  canAutoFit,
  disabled,
  onChange,
  onAutoFit,
}: {
  value: number
  autoFit: boolean
  canAutoFit: boolean
  disabled: boolean
  onChange: (value: number) => void
  onAutoFit: () => void
}) {
  const { t } = useTranslation()
  const id = useId()
  const [draft, setDraft] = useState<number | null>(value)

  useEffect(() => {
    setDraft(autoFit ? null : value)
  }, [autoFit, value])

  const select = (choice: string) => {
    if (choice === 'auto') {
      setDraft(null)
      onAutoFit()
      return
    }
    const size = Number(choice)
    if (Number.isFinite(size) && size > 0 && size <= 300) {
      setDraft(size)
      onChange(size)
    }
  }

  const commit = (next: number | null) => {
    if (next !== null && next > 0 && next <= 300) {
      onChange(next)
    } else {
      setDraft(autoFit ? null : value)
    }
  }

  return (
    <NumberField
      id={id}
      name='font-size'
      className='min-w-0'
      disabled={disabled}
      value={draft}
      min={0.5}
      max={300}
      step={0.5}
      onValueChange={setDraft}
      onValueCommitted={commit}
    >
      <NumberFieldGroup>
        <NumberFieldInput
          data-testid='type-size'
          aria-label={t('inspector.fontSize')}
          placeholder={t('inspector.auto')}
          className='px-2 text-left'
        />
        <DropdownMenu>
          <DropdownMenuTrigger
            disabled={disabled}
            aria-label={t('inspector.chooseFontSize')}
            className='grid size-6 shrink-0 place-items-center border-l border-input text-muted-foreground outline-none hover:bg-muted hover:text-foreground focus-visible:bg-muted focus-visible:text-foreground disabled:pointer-events-none disabled:opacity-40'
          >
            <ChevronDown className='size-3' />
          </DropdownMenuTrigger>
          <DropdownMenuContent
            align='end'
            className='w-28 min-w-28 border border-border/50 p-0.5 shadow-sm ring-0'
          >
            <DropdownMenuRadioGroup value={autoFit ? 'auto' : String(value)} onValueChange={select}>
              {canAutoFit && (
                <DropdownMenuRadioItem value='auto' className='min-h-6 py-0.5 text-[10px]'>
                  {t('inspector.auto')}
                </DropdownMenuRadioItem>
              )}
              {fontSizePresets.map((size) => (
                <DropdownMenuRadioItem
                  key={size}
                  value={String(size)}
                  className='min-h-6 py-0.5 text-[10px] tabular-nums'
                >
                  {size}
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuContent>
        </DropdownMenu>
      </NumberFieldGroup>
    </NumberField>
  )
}

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
      return SquareDashed
    default:
      return ImageIcon
  }
}

function localizedLayerName(layer: Layer, index: number, t: TFunction): string {
  if (layer.name) return layer.name
  if (isTextLayer(layer)) return plainText(layer).trim() || t('layers.textName', { index: index + 1 })
  if (isBlockLayer(layer)) {
    return (
      plainText(layer.translation).trim() ||
      plainText(layer.source).trim() ||
      t('layers.textName', { index: index + 1 })
    )
  }
  return t(`layers.kinds.${layer.kind}`)
}
