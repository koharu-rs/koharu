'use client'

import { useMutation } from '@tanstack/react-query'
import { AlignCenter, AlignLeft, AlignRight, ChevronDown, Italic } from 'lucide-react'
import { useEffect, useId, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Choice } from '@/components/controls/Choice'
import { ColorWell } from '@/components/controls/ColorWell'
import { FontPicker } from '@/components/controls/FontPicker'
import { ContextBar } from '@/components/editor/ContextBar'
import { toHex, toRgba } from '@/lib/color'
import {
  expandLayerSelection,
  isBlockLayer,
  isTextLayer,
  letteringOf,
  walkLayers,
} from '@/lib/document'
import { readPage, send, useFonts, usePage, usePreferences } from '@/lib/queries'
import { setStyle, updateLayer } from '@/lib/scene'
import { useKoharuStore, type CanvasTool } from '@/lib/store'
import {
  commands,
  type Align,
  type CharKey,
  type CharStyle,
  type Direction,
  type FontFamily,
  type Layer,
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
import { NumberField, NumberFieldGroup, NumberFieldInput } from '@koharu/ui/components/number-field'
import { Toggle } from '@koharu/ui/components/toggle'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'
import { cn } from '@koharu/ui/lib/utils'

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
  const page = usePage().data
  const selectedIds = useKoharuStore((state) => state.selectedLayers)
  const defaultFont = usePreferences().data?.renderer.font_families?.[0] ?? FALLBACK_FONT
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
  // The newest style asked for shows on the first target until the page is read with it.
  const restyle = useMutation({
    mutationFn: (edit: { layer: LayerId; style: TextStyle; targets: Layer[]; patch: TextStyle }) =>
      setStyle(edit.targets, edit.patch),
    onSettled: readPage,
  })

  // While typing, what the canvas says its selected characters share; otherwise what every
  // character of the selected texts shares.
  const shared = useMemo(
    () => typing ?? sharedChars(targets.flatMap((layer) => letteringOf(layer) ?? [])),
    [typing, targets],
  )
  const drafted =
    current && restyle.isPending && restyle.variables.layer === current.id
      ? restyle.variables.style
      : null
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
    restyle.mutate({
      layer: current.id,
      style: { ...style, ...patch, chars: { ...chars, ...patch.chars } },
      targets,
      patch,
    })
  }

  /** A character setting: while typing, the canvas styles the selected characters, or holds it
   * for the next ones typed at a caret; otherwise every character of each target takes it. */
  const applyChars = (patch: CharStyle) => {
    if (!typing) return apply({ chars: patch })
    // A mark can't unset a value, so no border on some characters is a border no wide.
    const marked = patch.stroke === null ? { ...patch, stroke: undefined, stroke_width: 0 } : patch
    send(commands.styleTyping, marked)
  }

  /** Merges `patch` into each target's lettering: a text's own values, a block's translation's. */
  const update = (patch: { [key in string]: Value }) => {
    for (const layer of targets) {
      send(updateLayer, layer.id, isBlockLayer(layer) ? { translation: patch } : patch)
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
  // colour, then the paragraph; italic and vertical text are toggles, and the border's colour and
  // width close the row, as Clip Studio Paint's text tool shows a text's edge beside its colour.
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
      <Choice
        size='sm'
        value={mixed.has('weight') ? '' : String(weight)}
        choices={Object.fromEntries(weights.map((value) => [value, String(value)]))}
        label={t('inspector.fontWeight')}
        className='w-14'
        onChange={(next) => applyChars({ weight: Number(next) })}
      />
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
      <div className='w-15'>
        <PresetField
          name='font-size'
          testId='type-size'
          value={size}
          min={0.5}
          // Photoshop's largest type size.
          max={1296}
          presets={fontSizes}
          label={t('inspector.fontSize')}
          menuLabel={t('inspector.chooseFontSize')}
          auto={{
            label: t('inspector.auto'),
            active: autoFit,
            available: fitted !== null,
            onPick: () => fitted && update({ frame: { [fitted]: { fit: 'best' } } }),
          }}
          onChange={(next) => {
            applyChars({ size: next })
            // A typed size is the size the text draws at, as in Photoshop and Clip Studio Paint, so
            // a frame that shrinks or grows its text stops fitting it; a growing box keeps the size.
            if (fitted && fit !== 'none' && fit !== 'grow') {
              update({ frame: { [fitted]: { fit: 'none' } } })
            }
          }}
        />
      </div>
      <ColorWell
        label={t('inspector.textColor')}
        size='sm'
        value={mixed.has('fill') ? '' : chars.fill ? toHex(chars.fill) : BLACK}
        onChange={(color) => color && applyChars({ fill: toRgba(color) })}
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
          <Tooltip key={value}>
            <TooltipTrigger
              render={
                <button
                  type='button'
                  aria-label={label}
                  aria-pressed={align === value}
                  data-active={align === value}
                  disabled={!paragraph}
                  className='grid place-items-center rounded-[4px] text-muted-foreground hover:text-foreground data-[active=true]:bg-primary data-[active=true]:text-primary-foreground data-[active=true]:hover:bg-primary/90'
                  onClick={() => apply({ align: value })}
                />
              }
            >
              <Icon className='size-3' />
            </TooltipTrigger>
            <TooltipContent>{label}</TooltipContent>
          </Tooltip>
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
      <div role='group' aria-label={t('inspector.border')} className='flex items-center gap-1'>
        <ColorWell
          label={t('inspector.borderColor')}
          size='sm'
          allowTransparent
          value={strokeEnabled && stroke ? toHex(stroke) : null}
          onChange={(color) =>
            applyChars(
              color === null
                ? { stroke: null }
                : { stroke: toRgba(color), stroke_width: strokeWidth > 0 ? strokeWidth : 1.5 },
            )
          }
        />
        <div className='w-12'>
          <PresetField
            name='border-width'
            value={strokeWidth > 0 ? strokeWidth : 1.5}
            min={0.5}
            max={32}
            presets={borderWidths}
            label={t('inspector.borderWidth')}
            menuLabel={t('inspector.chooseBorderWidth')}
            onChange={(next) => applyChars({ stroke_width: next, stroke: stroke ?? toRgba(WHITE) })}
          />
        </div>
      </div>
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

/** The font sizes the size field lists, as Photoshop's options bar does. */
const fontSizes = [8, 9, 10, 12, 14, 16, 18, 20, 24, 28, 32, 36, 48, 64, 72, 96]
/** The border widths the width field lists. */
const borderWidths = [1, 2, 3, 4, 5, 6, 8, 10, 12, 16, 20, 24, 32]

/** A number typed in, in half steps, or picked from `presets` behind the arrow beside it, as
 * Photoshop's options bar sets a font size. A typed number applies on Enter or on leaving the
 * field, and one outside `min` to `max` goes back. `auto` is a choice above the presets that the
 * field shows by its label. */
function PresetField({
  name,
  value,
  min,
  max,
  presets,
  label,
  menuLabel,
  auto,
  testId,
  onChange,
}: {
  name: string
  /** `null` when the selection doesn't share one. */
  value: number | null
  min: number
  max: number
  presets: number[]
  label: string
  menuLabel: string
  auto?: { label: string; active: boolean; available: boolean; onPick: () => void }
  testId?: string
  onChange: (value: number) => void
}) {
  const id = useId()
  const field = useRef<HTMLDivElement>(null)
  const shown = auto?.active ? null : value
  const [draft, setDraft] = useState<number | null>(shown)

  useEffect(() => {
    setDraft(shown)
  }, [shown])

  const pick = (choice: string) => {
    if (choice === 'auto') {
      setDraft(null)
      auto?.onPick()
      return
    }
    const preset = Number(choice)
    setDraft(preset)
    onChange(preset)
  }

  // The chosen one shows as a select's chosen item does, its tick left out, which the field's
  // narrow list has no room for.
  const item =
    'pr-2 data-checked:bg-accent data-checked:text-accent-foreground [&>[data-slot=dropdown-menu-radio-item-indicator]]:hidden'

  const commit = (next: number | null) => {
    if (next !== null && next >= min && next <= max) onChange(next)
    else setDraft(shown)
  }

  return (
    <NumberField
      id={id}
      name={name}
      className='min-w-0'
      value={draft}
      min={min}
      max={max}
      step={0.5}
      onValueChange={setDraft}
      onValueCommitted={commit}
    >
      <Tooltip>
        <TooltipTrigger render={<NumberFieldGroup ref={field} />}>
          <NumberFieldInput
            data-testid={testId}
            aria-label={label}
            placeholder={auto?.active ? auto.label : undefined}
            className='px-1 text-left text-ellipsis'
          />
          <DropdownMenu>
            <DropdownMenuTrigger
              aria-label={menuLabel}
              className='grid h-full w-4 shrink-0 place-items-center border-l border-input text-muted-foreground outline-none hover:bg-muted hover:text-foreground focus-visible:bg-muted focus-visible:text-foreground'
            >
              <ChevronDown className='size-3' />
            </DropdownMenuTrigger>
            {/* Under the whole field and as wide, as a select's list opens, wider only for a
              label that needs it. */}
            <DropdownMenuContent anchor={field} align='start' className='min-w-fit'>
              <DropdownMenuRadioGroup
                value={auto?.active ? 'auto' : String(value)}
                onValueChange={pick}
              >
                {auto?.available && (
                  <DropdownMenuRadioItem value='auto' className={item}>
                    {auto.label}
                  </DropdownMenuRadioItem>
                )}
                {presets.map((preset) => (
                  <DropdownMenuRadioItem
                    key={preset}
                    value={String(preset)}
                    className={cn(item, 'tabular-nums')}
                  >
                    {preset}
                  </DropdownMenuRadioItem>
                ))}
              </DropdownMenuRadioGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        </TooltipTrigger>
        <TooltipContent>{label}</TooltipContent>
      </Tooltip>
    </NumberField>
  )
}
