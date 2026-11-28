'use client'

import { PaintRoller, X } from 'lucide-react'
import { useMemo } from 'react'
import { useHotkeys } from 'react-hotkeys-hook'
import { useTranslation } from 'react-i18next'

import { toHex } from '@/lib/color'
import { expandLayerSelection, isBlockLayer, isTextLayer, walkLayers } from '@/lib/document'
import { clearFormat, paint, useFormatPainter, type Copied } from '@/lib/formatPainter'
import { usePage, useProject } from '@/lib/queries'
import { useKoharuStore } from '@/lib/store'
import { faceOf } from '@/lib/styles'
import { Button } from '@koharu/ui/components/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'

/** The Format Painter's bar, at the canvas's foot while it's the tool: what to click first, then
 * the style it holds, drawn, with how to paste it, to copy another, to paste it onto every
 * selected text at once, and to drop it. */
export function FormatPainterBar() {
  const { t } = useTranslation()
  const tool = useKoharuStore((state) => state.tool)
  const copied = useFormatPainter((state) => state.copied)
  const page = usePage().data
  const selected = useKoharuStore((state) => state.selectedLayers)
  const targets = useMemo(() => {
    if (!page) return []
    const leaves = expandLayerSelection(page.layers, selected)
    return walkLayers(page.layers)
      .map(({ layer }) => layer)
      .filter((layer) => leaves.includes(layer.id) && (isTextLayer(layer) || isBlockLayer(layer)))
  }, [page, selected])
  const active = tool === 'format_painter'
  useHotkeys('escape', clearFormat, { enabled: active && copied !== null })
  if (!active) return null

  return (
    <div
      role='status'
      aria-label={t('tools.format_painter')}
      className='absolute bottom-3 left-1/2 z-20 flex max-w-[calc(100%-1.5rem)] -translate-x-1/2 items-center gap-2 rounded-xl border border-border bg-[var(--surface-floating)] py-1 pr-1 pl-2.5 text-[11px] shadow-[var(--shadow-toolrail)]'
    >
      {copied ? (
        <>
          <Preview copied={copied} />
          <span className='min-w-0 truncate'>
            {t('formatPainter.copied', { words: copied.words || t('formatPainter.empty') })}
          </span>
          <span className='shrink-0 text-muted-foreground'>{t('formatPainter.reload')}</span>
          {targets.length > 0 && (
            <Button
              type='button'
              size='sm'
              className='h-7 shrink-0 text-[11px]'
              onClick={() => void paint(targets)}
            >
              {t('formatPainter.applySelected', { count: targets.length })}
            </Button>
          )}
          <Tooltip>
            <TooltipTrigger
              render={
                <Button
                  type='button'
                  variant='ghost'
                  size='icon-sm'
                  aria-label={t('formatPainter.clear')}
                  className='shrink-0 text-muted-foreground hover:text-foreground'
                  onClick={clearFormat}
                />
              }
            >
              <X className='size-3.5' />
            </TooltipTrigger>
            <TooltipContent side='top'>
              {t('formatPainter.clear')}
              <span className='ml-2 opacity-60'>Esc</span>
            </TooltipContent>
          </Tooltip>
        </>
      ) : (
        <span className='flex items-center gap-2 py-1.5 pr-1.5 text-muted-foreground'>
          <PaintRoller className='size-3.5' />
          {t('formatPainter.pick')}
        </span>
      )}
    </div>
  )
}

/** "Aa" in the style held: its face, its fill, and its outline, on a grey either shows on. */
function Preview({ copied }: { copied: Copied }) {
  const styles = useProject().data?.meta.styles
  const chars = copied.style.chars ?? {}
  const face = faceOf(copied.text, styles)
  const stroke = chars.stroke && (chars.stroke_width ?? 1) > 0 ? toHex(chars.stroke) : null
  return (
    <span
      aria-hidden='true'
      className='grid h-7 w-9 shrink-0 place-items-center rounded-md bg-[#8a8a8a] text-[17px] leading-none'
      style={{
        fontFamily: chars.family ?? face.family ?? undefined,
        fontWeight: chars.weight ?? face.weight ?? 400,
        fontStyle: (chars.italic ?? face.italic) ? 'italic' : 'normal',
        color: toHex(chars.fill ?? [0, 0, 0, 1]),
        WebkitTextStroke: stroke ? `3px ${stroke}` : undefined,
        paintOrder: 'stroke fill',
      }}
    >
      Aa
    </span>
  )
}
