'use client'

import {
  Brush,
  createLucideIcon,
  Eraser,
  Hand,
  Lasso,
  Minus,
  MousePointer2,
  Pipette,
  Plus,
  SplinePointer,
  SquareDashed,
  Type,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { ColorWell } from '@/components/controls/ColorWell'
import { usePage } from '@/lib/queries'
import { shortcutLabel, toolKeys } from '@/lib/shortcuts'
import { maxBrushDiameter, MIN_BRUSH_DIAMETER, useKoharuStore, type CanvasTool } from '@/lib/store'
import { Button } from '@koharu/ui/components/button'
import {
  NumberField,
  NumberFieldDecrement,
  NumberFieldGroup,
  NumberFieldIncrement,
  NumberFieldInput,
} from '@koharu/ui/components/number-field'
import {
  Popover,
  PopoverContent,
  PopoverTitle,
  PopoverTrigger,
} from '@koharu/ui/components/popover'
import { Slider } from '@koharu/ui/components/slider'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'

/** Photoshop's Remove tool icon, the Spot Healing Brush's band-aid across the corner with a
 * sparkle, drawn from Lucide's band-aid and its sparkles' plus. */
const Remove = createLucideIcon('remove-tool', [
  [
    'rect',
    {
      x: '5',
      y: '9',
      width: '17',
      height: '9',
      rx: '2',
      transform: 'rotate(-45 13.5 13.5)',
      key: 'band',
    },
  ],
  ['path', { d: 'M10.5 9v9M16.5 9v9', transform: 'rotate(-45 13.5 13.5)', key: 'pad' }],
  [
    'path',
    {
      d: 'M12.5 12.5h.01M14.5 12.5h.01M12.5 14.5h.01M14.5 14.5h.01',
      transform: 'rotate(-45 13.5 13.5)',
      key: 'holes',
    },
  ],
  ['path', { d: 'M5 1.5v5M2.5 4h5', key: 'sparkle' }],
])

const tools = [
  ['select', MousePointer2],
  ['direct_select', SplinePointer],
  ['marquee', SquareDashed],
  ['lasso', Lasso],
  ['text', Type],
  ['draw', Brush],
  ['eraser', Eraser],
  ['color_picker', Pipette],
  ['remove', Remove],
  ['pan', Hand],
] as const satisfies ReadonlyArray<readonly [CanvasTool, typeof MousePointer2]>

export function ToolBar() {
  const { t } = useTranslation()
  const page = usePage().data
  const active = useKoharuStore((state) => state.tool)
  const brush = useKoharuStore((state) => state.brush)
  const setTool = useKoharuStore((state) => state.setTool)
  const setBrush = useKoharuStore((state) => state.setBrush)

  return (
    <aside className='absolute top-3 left-3 z-20 flex w-11 flex-col rounded-2xl border border-border bg-[var(--surface-floating)] p-1 shadow-[var(--shadow-toolrail)]'>
      <div className='flex flex-col items-center py-0.5'>
        {tools.map(([tool, Icon], index) => (
          <div key={tool} className='contents'>
            {index === tools.length - 1 && <span className='my-1 h-px w-5 bg-border' />}
            <Tooltip>
              <TooltipTrigger
                render={
                  <Button
                    type='button'
                    variant='ghost'
                    size='icon'
                    disabled={!page}
                    aria-label={t(`tools.${tool}`)}
                    data-active={active === tool}
                    className='relative text-muted-foreground hover:bg-foreground/[0.06] hover:text-foreground disabled:opacity-30 data-[active=true]:bg-accent data-[active=true]:text-accent-foreground'
                    onClick={() => setTool(tool)}
                  />
                }
              >
                <Icon className='size-4' />
              </TooltipTrigger>
              <TooltipContent side='right'>
                {t(`tools.${tool}`)}
                <span className='ml-2 opacity-60'>{shortcutLabel(toolKeys[tool])}</span>
              </TooltipContent>
            </Tooltip>
          </div>
        ))}
      </div>

      <div className='flex flex-col items-center gap-1 border-t border-border/80 py-1.5'>
        <ColorWell value={brush.color} onChange={(color) => setBrush({ ...brush, color })} />
        <BrushSize
          value={brush.diameter}
          onChange={(diameter) => setBrush({ ...brush, diameter })}
        />
      </div>
    </aside>
  )
}

function BrushSize({ value, onChange }: { value: number; onChange: (value: number) => void }) {
  const { t } = useTranslation()
  const roundedValue = Math.round(value)
  const page = usePage().data
  const maxSize = maxBrushDiameter(page?.size ?? { width: 2048, height: 2048 })
  // Photoshop's brush picker shows the tip over its size; the tip grows with the size, up to
  // its box at the page's largest brush.
  const tip = clamp(2 + (16 * value) / maxSize, 2, 18)

  return (
    <Popover>
      <Tooltip>
        <PopoverTrigger
          render={
            <TooltipTrigger
              render={
                <Button
                  type='button'
                  variant='ghost'
                  size='icon'
                  aria-label={t('tools.brushSizePixels', { size: roundedValue })}
                  className='h-auto w-8 flex-col gap-1 rounded-lg py-1.5 font-sans text-[11px] leading-none font-medium tracking-[-0.02em] text-muted-foreground tabular-nums hover:bg-foreground/[0.06] hover:text-foreground'
                />
              }
            >
              <span className='grid size-[18px] place-items-center'>
                <span className='rounded-full bg-current' style={{ width: tip, height: tip }} />
              </span>
              {roundedValue}
            </TooltipTrigger>
          }
        />
        <TooltipContent side='right'>
          {t('tools.brushSizeShort', { size: roundedValue })}
        </TooltipContent>
      </Tooltip>
      <PopoverContent side='right' align='center' sideOffset={8} className='w-48'>
        <div className='flex items-center justify-between gap-3'>
          <PopoverTitle className='text-[11px]'>{t('tools.brushSize')}</PopoverTitle>
          <NumberField
            min={MIN_BRUSH_DIAMETER}
            step={1}
            value={roundedValue}
            className='w-20'
            onValueChange={(next) => {
              if (next !== null) onChange(Math.max(next, MIN_BRUSH_DIAMETER))
            }}
          >
            <NumberFieldGroup className='h-7'>
              <NumberFieldDecrement aria-label={t('tools.decreaseBrushSize')}>
                <Minus />
              </NumberFieldDecrement>
              <NumberFieldInput aria-label={t('tools.brushSizeInput')} />
              <NumberFieldIncrement aria-label={t('tools.increaseBrushSize')}>
                <Plus />
              </NumberFieldIncrement>
            </NumberFieldGroup>
          </NumberField>
        </div>
        <Slider
          aria-label={t('tools.brushSize')}
          min={MIN_BRUSH_DIAMETER}
          max={maxSize}
          step={1}
          value={value}
          className='py-1 [&_[data-slot=slider-thumb]]:size-2.5'
          onValueChange={onChange}
        />
      </PopoverContent>
    </Popover>
  )
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, Number.isFinite(value) ? value : min))
}
