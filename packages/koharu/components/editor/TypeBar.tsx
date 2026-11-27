'use client'

import { useMutation } from '@tanstack/react-query'
import {
  AlignCenter,
  AlignLeft,
  AlignRight,
  Bold,
  ChevronDown,
  Italic,
  Pilcrow,
  RefreshCw,
  RemoveFormatting,
  Type,
  TypeOutline,
} from 'lucide-react'
import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from 'react'
import { useHotkeys } from 'react-hotkeys-hook'
import { useTranslation } from 'react-i18next'

import { Choice } from '@/components/controls/Choice'
import { ColorWell } from '@/components/controls/ColorWell'
import { FontPicker } from '@/components/controls/FontPicker'
import { RubyField } from '@/components/controls/RubyField'
import { ContextBar } from '@/components/editor/ContextBar'
import { toHex, toRgba } from '@/lib/color'
import {
  expandLayerSelection,
  isBlockLayer,
  isTextLayer,
  letteringOf,
  walkLayers,
} from '@/lib/document'
import { readPage, send, useFonts, usePage, useProject } from '@/lib/queries'
import { setFit, setStyle, updateLayer, updateStyle } from '@/lib/scene'
import { shortcutLabel, shortcuts } from '@/lib/shortcuts'
import { useKoharuStore, type CanvasTool } from '@/lib/store'
import {
  BOLD,
  KERNELS,
  SEMIBOLD,
  cellOf,
  faceIn,
  faceOf,
  heaviest,
  scriptOfText,
  usableFontWeights,
} from '@/lib/styles'
import {
  commands,
  type Align,
  type CharKey,
  type CharStyle,
  type Direction,
  type Emphasis,
  type FontFamily,
  type Kernel,
  type Layer,
  type LayerId,
  type Orientation,
  type Text,
  type TextStyle,
  type Value,
} from '@koharu/ipc/protocol'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@koharu/ui/components/dropdown-menu'
import { NumberField, NumberFieldGroup, NumberFieldInput } from '@koharu/ui/components/number-field'
import { ScrollArea } from '@koharu/ui/components/scroll-area'
import { Toggle } from '@koharu/ui/components/toggle'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'
import { cn } from '@koharu/ui/lib/utils'

/** The renderer's last font when no face names one. */
const FALLBACK_FONT = 'Arial'
const BLACK = '#000000'
const WHITE = '#FFFFFF'

/** The tools whose selection the bar follows; a brush's settings are on the tool rail. */
const TOOLS: CanvasTool[] = ['select', 'direct_select', 'text']

/** The selected texts' type settings in a bar beside them on the canvas, as Photoshop's Contextual
 * Task Bar shows a selection's type settings on the canvas: one row, centred below the texts, or
 * above when there's no room below, past the canvas's edge when the canvas is narrower.
 * Every setting a text's lettering has: the style it draws with, its direction, and its box's
 * fit. As Photoshop's Character panel, while the canvas types, a character setting styles the
 * selected characters, or the next ones typed at a caret; otherwise every character of every text
 * and block selected. A value the characters don't share shows empty. */
export function TypeBar() {
  const { t } = useTranslation()
  const tool = useKoharuStore((state) => state.tool)
  const page = usePage().data
  const selectedIds = useKoharuStore((state) => state.selectedLayers)
  const styles = useProject().data?.meta.styles
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
  // What the first text's kernel gives it where its own style sets nothing; a new text being typed
  // has no language yet, which reads as Latin.
  const face = useMemo(
    () => (lettering ? faceOf(lettering, styles) : faceIn('Latn', null, styles)),
    [lettering, styles],
  )
  const defaultFont = face.family ?? FALLBACK_FONT
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
  // A box and a path fit their text alike, a balloon's translation in a path; point text has no fit.
  const fits = targets.flatMap((layer) => {
    const frame = letteringOf(layer)?.frame
    return typeof frame === 'object' ? [(frame.box ?? frame.path)?.fit] : []
  })
  const fitted = fits.length > 0
  const autoFit = fitted && fits.every((fit) => fit === 'best')
  // Frames that don't share Auto share no size.
  const mixedFit = !autoFit && fits.includes('best')

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
  const italic = Boolean(chars.italic ?? face.italic)
  const weight = chars.weight ?? face.weight ?? 400
  const bold = !mixed.has('weight') && weight >= SEMIBOLD
  const weights = usableFontWeights(selectedFamily)
  // What the selected family has no face for, the renderer fakes.
  const faux = fauxStyles(selectedFamily, italic)
  const size = mixed.has('size') || mixedFit ? null : Math.round((chars.size ?? 24) * 100) / 100
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
  const shown =
    selectionBounds && (targets.length > 0 || typing) && TOOLS.includes(tool)
      ? selectionBounds
      : null
  // A text's kernel shows `+` while its own values or marks set what its kernel's face does, as
  // Photoshop marks a character style with overrides. A text with no kernel is antique's.
  const kernels = new Set(
    targets.map((layer): Kernel => letteringOf(layer)?.style?.kernel ?? 'antique'),
  )
  const kernel = kernels.size === 1 ? [...kernels][0] : undefined
  const own = (style: CharStyle | null | undefined) =>
    style?.family != null || style?.weight != null || style?.italic != null
  const overridden = targets.some((layer) => {
    const text = letteringOf(layer)
    return own(text?.style?.chars) || (text?.content ?? []).some((span) => own(span.attributes))
  })
  /** Redefines the first text's style from its own values, as Google Docs' Update 'Heading 1' to
   * match does, which every text of that style then takes. */
  const updateToMatch = () => {
    if (!lettering || !kernel) return
    const script = scriptOfText(lettering)
    const style = cellOf(kernel, script, styles)
    const chars = lettering.style?.chars
    send(updateStyle, targets, kernel, script, {
      family: chars?.family ?? style?.family ?? null,
      weight: chars?.weight ?? style?.weight ?? null,
      italic: chars?.italic ?? style?.italic ?? null,
    })
  }
  const toggleBold = () => applyChars({ weight: bold ? 400 : BOLD })
  const toggleItalic = () => applyChars({ italic: !italic })
  const styling = { enabled: shown !== null, preventDefault: true, useKey: true }
  useHotkeys(shortcuts.bold, toggleBold, styling)
  useHotkeys(shortcuts.italic, toggleItalic, styling)
  if (!shown) return null

  // Two rows, in the order of Photoshop's type options bar: the style, the font, its weight, and its
  // size above, the weight and the size as narrow as their numbers so the style's and the font's
  // names have room; below, bold, italic, and the text's colour, a letter over it as Word's font
  // colour button draws, then the edge, as Clip Studio Paint's Edge is a setting of its own with its
  // colour and width, the letter outlined over its colour, then the direction, how characters stand
  // in a column, ruby, and emphasis marks, and the alignment.
  // Each button group is one cell, its buttons sharing its width, as Pages sets B, I, and U; the rows
  // are as wide as each other: the font menu takes what the upper one leaves, and the lower one
  // stretches its groups to fill it.
  const cell =
    'flex h-6 gap-px rounded-md border border-input bg-background p-px *:h-full *:min-w-0 *:flex-1 *:rounded-[calc(var(--radius-md)-2px)] *:border-0'
  const toggle =
    'h-full min-w-0 px-0 text-muted-foreground data-[pressed]:bg-primary data-[pressed]:text-primary-foreground data-[pressed]:hover:bg-primary/90'
  return (
    // Every menu's arrow a size down from a field's, over the controls' own, as the bar is compact.
    <ContextBar
      bounds={shown}
      aria-label={t('inspector.type')}
      data-testid='type-bar'
      className='flex-col items-stretch gap-1.5 [&_.lucide-chevron-down]:size-2.5!'
    >
      <div className='flex gap-1.5'>
        {/* The style, as Google Docs' style menu: pick one, or redefine it from this text, or clear
        what this text sets apart from it. */}
        <DropdownMenu>
          <Tooltip>
            <TooltipTrigger
              render={
                <DropdownMenuTrigger
                  aria-label={t('styles.kernel')}
                  className='flex h-6 w-26 shrink-0 items-center justify-between gap-1 rounded-md border border-input bg-background pr-1.5 pl-1 text-[11px] outline-none hover:bg-muted focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50'
                />
              }
            >
              {/* A pilcrow on a tile, as the script menu in Settings shows a script's letters, so
                the style reads apart from the face's menus beside it. */}
              <span className='flex min-w-0 items-center gap-1'>
                <span className='grid size-4 shrink-0 place-items-center rounded-xs bg-primary/10 text-primary'>
                  <Pilcrow className='size-2.5' />
                </span>
                <span className='truncate'>
                  {kernel ? `${t(`styles.kernels.${kernel}`)}${overridden ? '+' : ''}` : ''}
                </span>
              </span>
              <ChevronDown className='size-3 shrink-0 text-muted-foreground' />
            </TooltipTrigger>
            <TooltipContent>{t('styles.kernel')}</TooltipContent>
          </Tooltip>
          {/* As wide as its trigger, wider only for a label that needs it, as the weight's list. */}
          <DropdownMenuContent align='start' className={`min-w-fit ${gutter}`}>
            {/* Above the styles, which scroll in a list of their own below, so its scrollbar runs
              beside them alone; always there, dimmed while the text sets nothing of its own, as a
              menu shows a command it can't run, so they're found before they're needed. */}
            {kernel && (
              <>
                <DropdownMenuItem
                  className='text-[11px]'
                  disabled={!own(lettering?.style?.chars)}
                  onClick={updateToMatch}
                >
                  <RefreshCw className='size-3 text-muted-foreground' />
                  {t('styles.updateToMatch', { name: t(`styles.kernels.${kernel}`) })}
                </DropdownMenuItem>
                <DropdownMenuItem
                  className='text-[11px]'
                  disabled={!overridden}
                  onClick={() => apply({ chars: { family: null, weight: null, italic: null } })}
                >
                  {/* Clear formatting's Tx, as Word and Google Docs draw it. */}
                  <RemoveFormatting className='size-3 text-muted-foreground' />
                  {t('styles.clearOverrides')}
                </DropdownMenuItem>
                <DropdownMenuSeparator />
              </>
            )}
            <ScrollArea viewportClassName='max-h-60'>
              <DropdownMenuRadioGroup
                value={kernel ?? ''}
                // Picking a style clears the values it sets, as InDesign applies a character style
                // with Clear Overrides; while typing it still goes to the whole text.
                onValueChange={(next) =>
                  apply({
                    kernel: next as Kernel,
                    chars: { family: null, weight: null, italic: null },
                  })
                }
              >
                {KERNELS.map((value) => (
                  <DropdownMenuRadioItem key={value} value={value} className={item}>
                    {t(`styles.kernels.${value}`)}
                  </DropdownMenuRadioItem>
                ))}
              </DropdownMenuRadioGroup>
            </ScrollArea>
          </DropdownMenuContent>
        </DropdownMenu>
        {/* A flex box, so the inline trigger sets no line box taller than the row's other fields. */}
        <div className='flex w-40 min-w-0 grow'>
          <FontPicker
            value={family}
            families={families}
            size='sm'
            onChange={(next) => applyChars({ family: next })}
          />
        </div>
        <Choice
          size='sm'
          value={mixed.has('weight') ? '' : String(nearestFontWeight(weights, weight))}
          choices={Object.fromEntries(weights.map((value) => [value, String(value)]))}
          label={t('inspector.fontWeight')}
          className='w-12 shrink-0'
          onChange={(next) => applyChars({ weight: Number(next) })}
        />
        <div className='w-12 shrink-0'>
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
              available: fitted,
              onPick: () => send(setFit, targets, 'best'),
            }}
            onChange={(next) => {
              applyChars({ size: next })
              // A typed size is the size the text draws at, as in Photoshop and Clip Studio Paint, so
              // a frame that shrinks or grows its text stops fitting it; a growing box keeps the size.
              if (fitted) send(setFit, targets, 'none')
            }}
          />
        </div>
      </div>
      <div className='flex gap-1.5'>
        <div className={cn(cell, 'w-20 grow')}>
          {(
            [
              [
                shortcuts.bold,
                Bold,
                faux.bold ? t('inspector.fontStyles.fauxBold') : t('inspector.fontStyles.bold'),
                bold,
                toggleBold,
              ],
              [
                shortcuts.italic,
                Italic,
                faux.italic
                  ? t('inspector.fontStyles.fauxItalic')
                  : t('inspector.fontStyles.italic'),
                italic,
                toggleItalic,
              ],
            ] as const
          ).map(([shortcut, Icon, label, pressed, onToggle]) => (
            <Tooltip key={shortcut}>
              <TooltipTrigger
                render={
                  <Toggle
                    size='sm'
                    aria-label={label}
                    className={toggle}
                    pressed={pressed}
                    onPressedChange={onToggle}
                  />
                }
              >
                <Icon className='size-3.5' />
              </TooltipTrigger>
              <TooltipContent>
                {label}
                <span className='ml-2 opacity-60'>{shortcutLabel(shortcut)}</span>
              </TooltipContent>
            </Tooltip>
          ))}
          <ColorWell
            label={t('inspector.textColor')}
            size='sm'
            glyph={<Type className='size-3.5 text-foreground' />}
            value={mixed.has('fill') ? '' : chars.fill ? toHex(chars.fill) : BLACK}
            onChange={(color) => color && applyChars({ fill: toRgba(color) })}
          />
        </div>
        {/* The edge's colour a button wide, and its width the rest. */}
        <div
          role='group'
          aria-label={t('inspector.border')}
          className={cn(cell, 'grid w-20 grow grid-cols-[1.75rem_1fr] *:w-auto')}
        >
          <ColorWell
            label={t('inspector.borderColor')}
            size='sm'
            allowTransparent
            glyph={<TypeOutline strokeWidth={1.5} className='size-3.5 text-foreground' />}
            value={strokeEnabled && stroke ? toHex(stroke) : null}
            onChange={(color) =>
              applyChars(
                color === null
                  ? { stroke: null }
                  : { stroke: toRgba(color), stroke_width: strokeWidth > 0 ? strokeWidth : 1.5 },
              )
            }
          />
          <PresetField
            name='border-width'
            value={strokeWidth > 0 ? strokeWidth : 1.5}
            min={0.5}
            max={32}
            presets={borderWidths}
            label={t('inspector.borderWidth')}
            menuLabel={t('inspector.chooseBorderWidth')}
            className='h-5 rounded-[4px] border-0 bg-transparent dark:bg-transparent'
            onChange={(next) => applyChars({ stroke_width: next, stroke: stroke ?? toRgba(WHITE) })}
          />
        </div>
        <div className={cn(cell, 'w-28 grow')}>
          <Tooltip>
            <TooltipTrigger
              render={
                <Toggle
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
          {/* How the selected characters stand in a column, as Clip Studio Paint's TateChuYoko
            and CSS's text-orientation set them; horizontal text has no use for it. */}
          <SampleMenu
            label={t('inspector.orientation')}
            values={ORIENTATIONS}
            value={mixed.has('orientation') ? null : (chars.orientation ?? 'mixed')}
            unset='mixed'
            disabled={direction !== 'vertical'}
            name={(value) =>
              value === 'mixed' ? t('inspector.auto') : t(`inspector.orientations.${value}`)
            }
            sample={(value, compact) => <OrientationSample value={value} compact={compact} />}
            onPick={(orientation) => applyChars({ orientation })}
          />
          <RubyField typing={typing} />
          {/* The mark beside each selected character, as InDesign's Kenten lists them. */}
          <SampleMenu
            label={t('inspector.emphasis')}
            values={EMPHASES}
            value={mixed.has('emphasis') ? null : (chars.emphasis ?? 'none')}
            unset='none'
            name={(value) => t(`inspector.emphasisMarks.${value}`)}
            sample={(value, compact) => <EmphasisSample value={value} compact={compact} />}
            onPick={(emphasis) => applyChars({ emphasis })}
          />
        </div>
        <div role='group' aria-label={t('inspector.alignment')} className={cn(cell, 'w-18 grow')}>
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
  'orientation',
  'emphasis',
]

const ORIENTATIONS: Orientation[] = ['mixed', 'upright', 'sideways', 'tate_chu_yoko']
/** CSS's marks in its order, each filled and then open. */
const EMPHASES: Emphasis[] = [
  'none',
  'dot',
  'open_dot',
  'circle',
  'open_circle',
  'double_circle',
  'open_double_circle',
  'triangle',
  'open_triangle',
  'sesame',
  'open_sesame',
]

/** A small column showing how `value` stands its characters, set by the browser's own CSS; smaller
 * still when `compact`, as a cell's button shows it. Upright's two letters are stacked as lines,
 * since CSS's upright gives each a full vertical advance, which stood them half again as tall as
 * the other samples; tate-chu-yoko's two digits share a cell under a kana, so they read as set
 * across a column rather than as a plain number. */
function OrientationSample({ value, compact }: { value: Orientation; compact: boolean }) {
  const [lead, text, style] = {
    mixed: ['', 'あA', {}],
    upright: [
      '',
      'A\nB',
      {
        display: 'inline-block',
        writingMode: 'horizontal-tb',
        whiteSpace: 'pre',
        lineHeight: 0.85,
        textAlign: 'center',
      },
    ],
    sideways: ['', 'AB', { textOrientation: 'sideways' }],
    tate_chu_yoko: ['あ', '12', { textCombineUpright: 'all' }],
  }[value] as [string, string, CSSProperties]
  return (
    <span
      aria-hidden='true'
      className={cn(
        'leading-none [writing-mode:vertical-rl]',
        compact ? 'text-[8px]' : 'text-[9px]',
      )}
    >
      {lead}
      <span style={style}>{text}</span>
    </span>
  )
}

/** A character with `value`'s mark on its right, as a column sets it, by the browser's own CSS;
 * smaller when `compact`, as a cell's button shows it. */
function EmphasisSample({ value, compact }: { value: Emphasis; compact: boolean }) {
  const shape = value.replace('open_', '').replace('_', '-')
  const mark =
    value === 'none' ? 'none' : `${value.startsWith('open_') ? 'open' : 'filled'} ${shape}`
  return (
    <span
      aria-hidden='true'
      style={{ textEmphasis: mark, textEmphasisPosition: 'over right' }}
      className={cn(
        'leading-none [writing-mode:vertical-rl]',
        compact ? 'text-[8px]' : 'text-[11px]',
      )}
    >
      字
    </span>
  )
}

/** A button of a cell that picks one of `values` from a menu, each shown by its sample and its
 * name, as a select does; it shows the chosen one's sample, compact, and is pressed for any but
 * `unset`. A `null` value is one the selection doesn't share, which shows `unset`'s sample and picks
 * none. */
function SampleMenu<T extends string>({
  label,
  values,
  value,
  unset,
  disabled,
  name,
  sample,
  onPick,
}: {
  label: string
  values: T[]
  value: T | null
  unset: T
  disabled?: boolean
  name: (value: T) => string
  sample: (value: T, compact: boolean) => ReactNode
  onPick: (value: T) => void
}) {
  return (
    <DropdownMenu>
      <Tooltip>
        <TooltipTrigger
          render={
            <DropdownMenuTrigger
              aria-label={label}
              disabled={disabled}
              data-active={value !== null && value !== unset}
              className='flex items-center justify-center gap-px text-muted-foreground outline-none hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-40 data-[active=true]:bg-primary data-[active=true]:text-primary-foreground data-[active=true]:hover:bg-primary/90'
            />
          }
        >
          {sample(value ?? unset, true)}
          <ChevronDown className='shrink-0' />
        </TooltipTrigger>
        <TooltipContent>
          {label}
          {value !== null && <span className='ml-2 opacity-60'>{name(value)}</span>}
        </TooltipContent>
      </Tooltip>
      <DropdownMenuContent align='start' className={list}>
        <DropdownMenuRadioGroup value={value ?? ''} onValueChange={(next) => onPick(next as T)}>
          {values.map((option) => (
            <DropdownMenuRadioItem key={option} value={option} className={cn(item, 'gap-2')}>
              {/* Each sample starts at the column's start, so a character without a mark stands
                where the marked ones do. */}
              <span className='grid w-4 shrink-0 items-center justify-items-start'>
                {sample(option, false)}
              </span>
              {name(option)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

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

/** Whether `family` has no bold face for text as upright or `italic` as it is, and no italic one:
 * a text takes a face of its slant first. */
function fauxStyles(family: FontFamily | undefined, italic: boolean) {
  if (!family) return { bold: false, italic: false }
  const styled = family.faces.filter((face) => face.italic === italic)
  return {
    bold: !(styled.length ? styled : family.faces).some((face) => heaviest(face) >= SEMIBOLD),
    italic: !family.faces.some((face) => face.italic),
  }
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
/** A bar's menu item that picks a value: the chosen one shows as a select's chosen item does, its
 * tick left out, which a list as narrow as its field has no room for. */
const item =
  'pr-2 data-checked:bg-accent data-checked:text-accent-foreground [&>[data-slot=dropdown-menu-radio-item-indicator]]:hidden'
/** While a menu's list scrolls, its items stop short of the scrollbar, which ScrollArea draws over
 * what it scrolls. */
const gutter =
  '[&_[data-slot=scroll-area][data-has-overflow-y]_[data-slot=scroll-area-content]]:pr-2.5'
/** A bar's menu of such items: as wide as what opens it, wider only for a label that needs it, and no
 * taller than 20rem, which caps the scroll area itself, as the menu's box would only clip it. */
const list = `min-w-fit [&_[data-slot=scroll-area-viewport]]:max-h-[min(20rem,var(--available-height))] ${gutter}`

/** A number typed in, in half steps, or picked from `presets` behind the arrow beside it, as
 * Photoshop's options bar sets a font size. A typed number applies on Enter or on leaving the
 * field, and one outside `min` to `max` goes back. `auto` is a choice above the presets that the
 * field shows by its label. `className` restyles its field, as a member of a joined cell draws no
 * outline of its own. */
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
  className,
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
  className?: string
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
        <TooltipTrigger render={<NumberFieldGroup ref={field} className={className} />}>
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
            <DropdownMenuContent anchor={field} align='start' className={list}>
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
