'use client'

import { Pipette, SquareSlash } from 'lucide-react'
import { useEffect, useRef, useState, type ReactNode } from 'react'
import { HexColorInput, HexColorPicker } from 'react-colorful'
import { useTranslation } from 'react-i18next'

import { useColorSampling } from '@/components/controls/ColorSampling'
import { toHex, toRgba } from '@/lib/color'
import { rememberColor, useColorHistory } from '@/lib/colorHistory'
import { Button } from '@koharu/ui/components/button'
import { Popover, PopoverContent, PopoverTrigger } from '@koharu/ui/components/popover'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'
import { cn } from '@koharu/ui/lib/utils'

type ColorWellProps = {
  label?: string
  size?: 'default' | 'sm'
  disabled?: boolean
  /** Drawn over a bar of the colour in place of a swatch, as Word's font colour button draws an A,
   * so wells side by side say what each colours. */
  glyph?: ReactNode
} & (
  | {
      value: string
      allowTransparent?: false
      onChange: (color: string) => void
    }
  | {
      value: string | null
      allowTransparent: true
      onChange: (color: string | null) => void
    }
)

export function ColorWell(props: ColorWellProps) {
  const { t } = useTranslation()
  const {
    value,
    label,
    size = 'default',
    disabled = false,
    allowTransparent = false,
    glyph,
  } = props
  const [draft, setDraft] = useState(value ?? '#000000')
  const [transparent, setTransparent] = useState(value === null)
  const [open, setOpen] = useState(false)
  const [typed, setTyped] = useState<{ text: string; color: string } | null>(null)
  const opened = useRef(draft)
  const sampling = useColorSampling()
  const recent = useColorHistory((state) => state.colors)
  const accessibleLabel = label ?? t('colorPicker.brushColor')

  useEffect(() => {
    if (value === null) {
      setTransparent(true)
    } else {
      setDraft(value)
      setTransparent(false)
    }
  }, [value])

  const set = (color: string) => {
    const normalized = toHex(toRgba(color))
    setDraft(normalized)
    setTransparent(false)
    if (props.allowTransparent) props.onChange(normalized)
    else props.onChange(normalized)
    return normalized
  }

  const clear = () => {
    if (!props.allowTransparent) return
    setTransparent(true)
    props.onChange(null)
  }

  // A colour joins the history when the popover closes on it, so the drags and the keystrokes of a
  // hex code that led there don't fill it; canvas picks join it in `ColorSamplingProvider`.
  const openChange = (next: boolean) => {
    if (next) opened.current = draft
    else if (!transparent && draft !== opened.current) rememberColor(draft)
    setOpen(next)
  }

  return (
    <Popover open={open} onOpenChange={openChange}>
      <Tooltip>
        <TooltipTrigger
          render={
            <PopoverTrigger
              aria-label={accessibleLabel}
              disabled={disabled}
              // A small well is a field among a bar's fields; a large one is a bare swatch, as
              // Photoshop's toolbar shows its foreground colour.
              className={cn(
                'grid place-items-center outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-40',
                size === 'sm'
                  ? 'size-6 rounded-md border border-input bg-background'
                  : 'size-8 rounded-md',
              )}
            />
          }
        >
          {glyph ? (
            <span className='flex flex-col items-center gap-px leading-none'>
              {glyph}
              {/* None shows as an empty bar. */}
              <span
                className={cn('h-[3px] w-3.5 rounded-[1px]', transparent && 'ring-1 ring-input')}
                style={transparent ? undefined : { backgroundColor: draft }}
              />
            </span>
          ) : transparent ? (
            <SquareSlash
              aria-hidden='true'
              className={cn('text-muted-foreground', size === 'sm' ? 'size-3.5' : 'size-4')}
            />
          ) : (
            <span
              className={cn(
                size === 'sm'
                  ? 'size-3 rounded-[3px] ring-1 ring-black/15'
                  : 'size-5 rounded-[6px] border border-foreground/40 shadow-[inset_0_0_0_1px_white]',
              )}
              style={{ backgroundColor: draft }}
            />
          )}
        </TooltipTrigger>
        {/* Beside the tool rail's tools, as theirs are. */}
        <TooltipContent side={size === 'sm' ? undefined : 'right'}>
          {accessibleLabel}
        </TooltipContent>
      </Tooltip>
      <PopoverContent side='right' align='start' className='w-60 p-3'>
        <div>
          <HexColorPicker
            color={draft}
            onChange={(color) => setDraft(toHex(toRgba(color)))}
            onChangeEnd={set}
          />
        </div>
        <div className='mt-3 flex gap-2'>
          {/* The field resets its text whenever `color` changes, so it keeps the text typed while
              that names the colour applied: the expanded colour would turn a typed `ff0` into
              `FFFF00` and cut off the code's remaining digits. */}
          <HexColorInput
            prefixed
            color={typed?.color === draft ? typed.text : draft}
            aria-label={t('colorPicker.hexCode')}
            className='h-8 min-w-0 flex-1 rounded-lg border border-input bg-background px-2 font-mono text-[11px] uppercase outline-none focus:border-ring'
            onChange={(text) => setTyped({ text, color: set(text) })}
          />
          {sampling && (
            <Button
              size='icon'
              variant='outline'
              aria-label={t('colorPicker.sampleCanvas')}
              onClick={() => {
                setOpen(false)
                sampling.request(set)
              }}
            >
              <Pipette />
            </Button>
          )}
        </div>
        {recent.length > 0 && (
          <div role='group' aria-label={t('colorPicker.recent')} className='mt-3 flex gap-1'>
            {recent.map((color) => (
              <button
                key={color}
                type='button'
                aria-label={t('colorPicker.recentColor', { color })}
                className='size-5 rounded-[3px] ring-1 ring-black/15 outline-none focus-visible:ring-2 focus-visible:ring-ring'
                style={{ backgroundColor: color }}
                onClick={() => set(color)}
              />
            ))}
          </div>
        )}
        {allowTransparent && (
          <Button
            type='button'
            size='sm'
            variant={transparent ? 'secondary' : 'ghost'}
            className='mt-2 w-full justify-start gap-2'
            onClick={clear}
          >
            <SquareSlash aria-hidden='true' className='size-3.5 text-muted-foreground' />
            {t('colorPicker.transparent')}
          </Button>
        )}
      </PopoverContent>
    </Popover>
  )
}
