'use client'

import {
  AlignCenter,
  AlignLeft,
  AlignRight,
  ChevronDown,
  Italic,
  Minus,
  Plus,
  SquareSlash,
} from 'lucide-react'
import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { ColorWell } from '@/components/controls/ColorWell'
import { FontPicker } from '@/components/controls/FontPicker'
import { call, dispatch } from '@/lib/backend'
import {
  expandLayerSelection,
  isBlockLayer,
  isTextLayer,
  letteringOf,
  walkLayers,
} from '@/lib/document'
import { cssBox, textBounds } from '@/lib/geometry'
import { useFonts, usePage } from '@/lib/queries'
import { setStyle, updateLayer } from '@/lib/scene'
import { hexToRgba, rgbaToHex, useKoharuStore, type CanvasTool } from '@/lib/store'
import {
  type Align,
  type Direction,
  type FontFamily,
  type LayerId,
  type Text,
  type TextStyle,
  type Value,
} from '@koharu/ipc/protocol'
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
import { Popover, PopoverContent, PopoverTrigger } from '@koharu/ui/components/popover'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@koharu/ui/components/select'
import { Toggle } from '@koharu/ui/components/toggle'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'

/** The renderer's own default when `[renderer]` names no font. */
const FALLBACK_FONT = 'CCWildWords'
const BLACK = '#000000'
const WHITE = '#FFFFFF'

interface Size {
  width: number
  height: number
}

/** The space between the bar and the texts, and the canvas's edges, in CSS pixels. */
const GAP = 8

/** The tools whose selection the bar follows; a brush's settings are on the tool rail. */
const TOOLS: CanvasTool[] = ['select', 'direct_select', 'text']

/** The selected texts' type settings in a bar beside them on the canvas, as Photoshop's Contextual
 * Task Bar shows a selection's type settings on the canvas: centred below the texts, or above when
 * there's no room below, kept inside the canvas, on one row or two when the canvas is narrower.
 * Every setting a text's lettering has: the style it draws with, its direction, and its box's
 * fit. A change goes to every text and block selected. */
export function TypeBar() {
  const { t } = useTranslation()
  const camera = useKoharuStore((state) => state.camera)
  const tool = useKoharuStore((state) => state.tool)
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
    void call(setStyle, targets, patch)
      .catch(() => undefined)
      .finally(() => {
        if (sequence.current === request) setDraft(null)
      })
  }

  /** Merges `patch` into each target's lettering: a text's own values, a block's translation's. */
  const update = (patch: { [key in string]: Value }) => {
    for (const layer of targets) {
      dispatch(updateLayer, layer.id, isBlockLayer(layer) ? { translation: patch } : patch)
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

  // Where the bar goes: below the texts, or above when there's no room below, inside the canvas.
  const area = useMemo(() => {
    const boxes = targets.flatMap((layer) => {
      const text = isBlockLayer(layer)
        ? (layer.translation ?? layer.source)
        : isTextLayer(layer)
          ? layer
          : null
      const bounds = text ? textBounds(text) : null
      return bounds ? [cssBox(bounds, camera)] : []
    })
    if (!boxes.length) return null
    const left = Math.min(...boxes.map((box) => box.x))
    const top = Math.min(...boxes.map((box) => box.y))
    const right = Math.max(...boxes.map((box) => box.x + box.width))
    const bottom = Math.max(...boxes.map((box) => box.y + box.height))
    return { x: left, y: top, width: right - left, height: bottom - top }
  }, [targets, camera])
  const shown = area !== null && TOOLS.includes(tool)
  const bar = useRef<HTMLDivElement>(null)
  // The bar's size and the canvas's, measured again whenever either changes, as when the bar
  // wraps to a second row or the window is resized.
  const [sizes, setSizes] = useState<{ bar: Size; room: Size } | null>(null)

  useLayoutEffect(() => {
    const element = bar.current
    const room = element?.parentElement
    if (!shown || !element || !room) return
    const measure = () =>
      setSizes({
        bar: { width: element.offsetWidth, height: element.offsetHeight },
        room: { width: room.clientWidth, height: room.clientHeight },
      })
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    observer.observe(room)
    return () => observer.disconnect()
  }, [shown])

  if (!shown) return null
  const place = sizes && {
    top:
      area.y + area.height + GAP + sizes.bar.height <= sizes.room.height - GAP
        ? area.y + area.height + GAP
        : Math.max(
            GAP,
            Math.min(area.y - GAP - sizes.bar.height, sizes.room.height - GAP - sizes.bar.height),
          ),
    left: Math.max(
      GAP,
      Math.min(
        area.x + area.width / 2 - sizes.bar.width / 2,
        sizes.room.width - GAP - sizes.bar.width,
      ),
    ),
  }

  // One row, as Photoshop's type options bar, at the user's choice: the face, its size, and its
  // colour, then the paragraph; italic and vertical text are toggles, and the border sits behind
  // one button, as Photoshop keeps a stroke out of the bar.
  const toggle =
    'h-6 w-6 min-w-6 rounded-md bg-background px-0 text-muted-foreground data-[pressed]:border-primary data-[pressed]:bg-primary data-[pressed]:text-primary-foreground data-[pressed]:hover:bg-primary/90'
  return (
    <div
      ref={bar}
      role='toolbar'
      aria-label={t('inspector.type')}
      data-testid='type-bar'
      className='absolute z-20 flex max-w-[calc(100%-16px)] flex-wrap items-center gap-1 rounded-xl border border-border bg-[var(--surface-floating)] p-1.5 shadow-[var(--shadow-toolrail)]'
      style={{
        left: place?.left ?? 0,
        top: place?.top ?? 0,
        visibility: place ? 'visible' : 'hidden',
      }}
    >
      <div className='w-28'>
        <FontPicker
          value={family}
          families={families}
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
      </div>
      <Select
        value={String(weight)}
        onValueChange={(next) => next && apply({ chars: { weight: Number(next) } })}
      >
        <SelectTrigger size='sm' aria-label={t('inspector.fontWeight')} className='w-16'>
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
      <Tooltip>
        <TooltipTrigger
          render={
            <Toggle
              variant='outline'
              size='sm'
              aria-label={t('inspector.fontStyles.italic')}
              className={toggle}
              disabled={!italics && !italic}
              pressed={italic}
              onPressedChange={(nextItalic) =>
                apply({
                  chars: {
                    italic: nextItalic,
                    weight: nearestFontWeight(
                      usableFontWeights(selectedFamily, nextItalic),
                      weight,
                    ),
                  },
                })
              }
            />
          }
        >
          <Italic className='size-3.5' />
        </TooltipTrigger>
        <TooltipContent>{t('inspector.fontStyles.italic')}</TooltipContent>
      </Tooltip>
      <div className='w-[4.5rem]'>
        <FontSizeField
          disabled={false}
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
      </div>
      <ColorWell
        label={t('inspector.textColor')}
        size='sm'
        value={chars.fill ? rgbaToHex(chars.fill) : BLACK}
        onChange={(color) => color && apply({ chars: { fill: hexToRgba(color) } })}
      />
      <div
        role='group'
        aria-label={t('inspector.alignment')}
        className='grid h-6 w-[4.5rem] grid-cols-3 rounded-md border border-input bg-background p-px'
      >
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
              direction === 'vertical' ? t('inspector.alignMiddle') : t('inspector.alignCenter'),
            ],
            [
              'end',
              AlignRight,
              direction === 'vertical' ? t('inspector.alignBottom') : t('inspector.alignRight'),
            ],
          ] as const
        ).map(([value, Icon, label]) => (
          <button
            key={value}
            type='button'
            aria-label={label}
            aria-pressed={align === value}
            data-active={align === value}
            className='grid place-items-center rounded-[4px] text-muted-foreground hover:text-foreground data-[active=true]:bg-primary data-[active=true]:text-primary-foreground data-[active=true]:hover:bg-primary/90'
            onClick={() => apply({ align: value })}
          >
            <Icon className='size-3' />
          </button>
        ))}
      </div>
      <Tooltip>
        <TooltipTrigger
          render={
            <Toggle
              variant='outline'
              size='sm'
              aria-label={t('inspector.vertical')}
              className={toggle}
              pressed={direction === 'vertical'}
              onPressedChange={(vertical) =>
                update({ direction: vertical ? 'vertical' : 'horizontal' })
              }
            />
          }
        >
          {/* Columns of text read from the right, as vertical text sets. */}
          <svg
            aria-hidden='true'
            viewBox='0 0 24 24'
            fill='none'
            stroke='currentColor'
            strokeWidth={2}
            strokeLinecap='round'
            className='size-3.5'
          >
            <path d='M18 4v16M12 4v11M6 4v7' />
          </svg>
        </TooltipTrigger>
        <TooltipContent>
          {t('inspector.textDirection')}
          <span className='ml-2 opacity-60'>{t(`inspector.${direction}`)}</span>
        </TooltipContent>
      </Tooltip>
      <Popover>
        <Tooltip>
          <TooltipTrigger
            render={
              <PopoverTrigger
                aria-label={t('inspector.border')}
                className='grid size-6 shrink-0 place-items-center rounded-md border border-input bg-background text-muted-foreground hover:bg-muted'
              />
            }
          >
            {strokeEnabled && stroke ? (
              <span
                aria-hidden='true'
                className='size-3 rounded-[3px] border-2'
                style={{ borderColor: rgbaToHex(stroke) }}
              />
            ) : (
              <SquareSlash aria-hidden='true' className='size-3.5' />
            )}
          </TooltipTrigger>
          <TooltipContent>{t('inspector.border')}</TooltipContent>
        </Tooltip>
        <PopoverContent side='bottom' align='end' className='w-auto gap-1.5 rounded-xl p-2'>
          <span className='text-[10px] font-medium text-muted-foreground'>
            {t('inspector.border')}
          </span>
          <div className='flex items-center gap-1'>
            <ColorWell
              label={t('inspector.borderColor')}
              size='sm'
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
            <NumberField
              id={borderWidthId}
              name='border-width'
              className='w-24'
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
          </div>
        </PopoverContent>
      </Popover>
    </div>
  )
}

/** A family the font list hasn't loaded, so the default still shows. */
function placeholderFamily(name: string): FontFamily {
  return {
    name,
    faces: [{ post_script_name: name, weight: 400, weight_range: null, italic: false }],
  }
}

function hasItalic(family: FontFamily | undefined): boolean {
  return Boolean(family?.faces.some((face) => face.italic))
}

function usableFontWeights(family: FontFamily | undefined, italic: boolean): number[] {
  if (!family) return [400]
  const styled = family.faces.filter((face) => face.italic === italic)
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
    (nearest, weight) =>
      Math.abs(weight - target) < Math.abs(nearest - target) ? weight : nearest,
    weights[0] ?? target,
  )
}

function sameFont(left: string, right: string): boolean {
  const normalize = (value: string) =>
    value.normalize('NFKC').trim().replace(/\s+/g, ' ').toLowerCase()
  return normalize(left) === normalize(right)
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
          className='px-1.5 text-left text-ellipsis'
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
