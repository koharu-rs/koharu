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
import { useEffect, useId, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { ColorWell } from '@/components/controls/ColorWell'
import { FontPicker } from '@/components/controls/FontPicker'
import { ContextBar } from '@/components/editor/ContextBar'
import { call, dispatch } from '@/lib/backend'
import {
  expandLayerSelection,
  isBlockLayer,
  isTextLayer,
  letteringOf,
  walkLayers,
} from '@/lib/document'
import { useFonts, usePage } from '@/lib/queries'
import { setStyle, updateLayer } from '@/lib/scene'
import { hexToRgba, rgbaToHex, useKoharuStore, type CanvasTool } from '@/lib/store'
import {
  commands,
  type Align,
  type CharKey,
  type CharStyle,
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

/** The tools whose selection the bar follows; a brush's settings are on the tool rail. */
const TOOLS: CanvasTool[] = ['select', 'direct_select', 'text']

/** The selected texts' type settings in a bar beside them on the canvas, as Photoshop's Contextual
 * Task Bar shows a selection's type settings on the canvas: centred below the texts, or above when
 * there's no room below, kept inside the canvas, on one row or two when the canvas is narrower.
 * Every setting a text's lettering has: the style it draws with, its direction, and its box's
 * fit. As Photoshop's Character panel, while the canvas types, a character setting styles the
 * selected characters, or the next ones typed at a caret; otherwise every character of every text
 * and block selected. A value the characters don't share shows empty. */
export function TypeBar() {
  const { t } = useTranslation()
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
  const typing = useKoharuStore((state) => state.typing)
  const [draft, setDraft] = useState<{ layer: LayerId; style: TextStyle } | null>(null)
  const sequence = useRef(0)

  useEffect(() => setDraft(null), [current?.id])

  // While typing, what the canvas says its selected characters share; otherwise what every
  // character of the selected texts shares.
  const shared = useMemo(
    () => typing ?? sharedChars(targets.flatMap((layer) => letteringOf(layer) ?? [])),
    [typing, targets],
  )
  const drafted = current && draft?.layer === current.id ? draft.style : null
  const style: TextStyle = {
    ...lettering?.style,
    ...drafted,
    chars: { ...shared.chars, ...drafted?.chars },
  }
  const chars = style.chars ?? {}
  const mixed = new Set(shared.mixed.filter((key) => drafted?.chars?.[key] === undefined))
  const frame = lettering?.frame ?? 'point'
  // A box and a path fit their text alike, a balloon's translation in a path; point text has no fit.
  const fitted = typeof frame !== 'object' ? null : frame.box ? 'box' : 'path'
  const fit = typeof frame === 'object' ? (frame.box ?? frame.path)?.fit : undefined
  const autoFit = fit === 'best'

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

  /** A character setting: while typing, the canvas styles the selected characters, or holds it
   * for the next ones typed at a caret; otherwise every character of each target takes it. */
  const applyChars = (patch: CharStyle) => {
    if (!typing) return apply({ chars: patch })
    // A mark can't unset a value, so no border on some characters is a border no wide.
    const marked = patch.stroke === null ? { ...patch, stroke: undefined, stroke_width: 0 } : patch
    void call(commands.styleTyping, marked).catch(() => undefined)
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
  const family = mixed.has('family') ? '' : (chars.family ?? defaultFont)
  const selectedFamily = families.find((candidate) => sameFont(candidate.name, family))
  const italics = hasItalic(selectedFamily)
  const italic = Boolean(chars.italic) && italics
  const weight = chars.weight ?? 400
  const weights = usableFontWeights(selectedFamily, italic)
  const size = mixed.has('size') ? null : Math.round((chars.size ?? 24) * 100) / 100
  const strokeWidth = chars.stroke_width ?? 0
  const stroke = chars.stroke ?? null
  const strokeEnabled = stroke !== null && stroke[3] > 0 && strokeWidth > 0
  const direction: Direction = lettering?.direction ?? 'horizontal'
  const align: Align = style.align ?? (direction === 'vertical' ? 'start' : 'center')

  // The bar sits by the selection as the canvas lays it out, a point text's lines and the text
  // being typed included.
  const selectionBounds = useKoharuStore((state) => state.selectionBounds)
  // A new text being typed has no layer yet for its paragraph to go to.
  const paragraph = targets.length > 0
  if (!selectionBounds || !(targets.length || typing) || !TOOLS.includes(tool)) return null

  // One row, as Photoshop's type options bar, at the user's choice: the face, its size, and its
  // colour, then the paragraph; italic and vertical text are toggles, and the border sits behind
  // one button, as Photoshop keeps a stroke out of the bar.
  const toggle =
    'h-6 w-6 min-w-6 rounded-md bg-background px-0 text-muted-foreground data-[pressed]:border-primary data-[pressed]:bg-primary data-[pressed]:text-primary-foreground data-[pressed]:hover:bg-primary/90'
  return (
    <ContextBar
      bounds={selectionBounds}
      aria-label={t('inspector.type')}
      data-testid='type-bar'
      className='flex-wrap'
    >
      <div className='w-28'>
        <FontPicker
          value={family}
          families={families}
          size='sm'
          onChange={(next) => {
            const nextFamily = families.find((candidate) => sameFont(candidate.name, next))
            const nextItalic = italic && hasItalic(nextFamily)
            applyChars({
              family: next,
              italic: nextItalic,
              weight: nearestFontWeight(usableFontWeights(nextFamily, nextItalic), weight),
            })
          }}
        />
      </div>
      <Select
        value={mixed.has('weight') ? '' : String(weight)}
        onValueChange={(next) => next && applyChars({ weight: Number(next) })}
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
                applyChars({
                  italic: nextItalic,
                  weight: nearestFontWeight(usableFontWeights(selectedFamily, nextItalic), weight),
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
          canAutoFit={fitted !== null}
          onChange={(next) => {
            applyChars({ size: next })
            // A typed size is the size the text draws at, as in Photoshop and Clip Studio Paint, so
            // a frame that shrinks or grows its text stops fitting it; a growing box keeps the size.
            if (fitted && fit !== 'none' && fit !== 'grow') {
              update({ frame: { [fitted]: { fit: 'none' } } })
            }
          }}
          onAutoFit={() => fitted && update({ frame: { [fitted]: { fit: 'best' } } })}
        />
      </div>
      <ColorWell
        label={t('inspector.textColor')}
        size='sm'
        value={mixed.has('fill') ? '' : chars.fill ? rgbaToHex(chars.fill) : BLACK}
        onChange={(color) => color && applyChars({ fill: hexToRgba(color) })}
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
            disabled={!paragraph}
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
              disabled={!paragraph}
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
                applyChars(
                  color === null
                    ? { stroke: null }
                    : {
                        stroke: hexToRgba(color),
                        stroke_width: strokeWidth > 0 ? strokeWidth : 1.5,
                      },
                )
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
                  applyChars({ stroke_width: next, stroke: stroke ?? hexToRgba(WHITE) })
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
    </ContextBar>
  )
}

/** A family the font list hasn't loaded, so the default still shows. */
function placeholderFamily(name: string): FontFamily {
  return {
    name,
    faces: [{ post_script_name: name, weight: 400, weight_range: null, italic: false }],
  }
}

const CHAR_KEYS: CharKey[] = [
  'family',
  'weight',
  'italic',
  'size',
  'fill',
  'stroke',
  'stroke_width',
]

/** What every character of `texts` shares, each span's marks over its text's style, and the values
 * they don't, which it leaves empty, as the canvas reports a typed selection's. */
function sharedChars(texts: Text[]): { chars: CharStyle; mixed: CharKey[] } {
  const styles: CharStyle[] = texts.flatMap((text) => {
    const base = text.style?.chars ?? {}
    const spans = text.content?.length ? text.content : [{ insert: '' }]
    return spans.map((span) => ({
      ...base,
      ...Object.fromEntries(
        Object.entries(span.attributes ?? {}).filter(([, value]) => value != null),
      ),
    }))
  })
  const [first, ...rest] = styles
  if (!first) return { chars: {}, mixed: [] }
  const same = (a: unknown, b: unknown) => JSON.stringify(a ?? null) === JSON.stringify(b ?? null)
  const mixed = CHAR_KEYS.filter((key) => rest.some((style) => !same(style[key], first[key])))
  const chars: CharStyle = { ...first }
  for (const key of mixed) delete chars[key]
  return { chars, mixed }
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
  /** `null` when the characters don't share one. */
  value: number | null
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
          placeholder={autoFit ? t('inspector.auto') : undefined}
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
