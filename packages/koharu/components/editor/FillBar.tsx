'use client'

import { Sparkles, X } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { StageModelSelect } from '@/components/controls/ModelPicker'
import { ContextBar } from '@/components/editor/ContextBar'
import { computeOtherStagesVram, inpainters } from '@/lib/models'
import { send, useModelCatalog, usePreferences, useSavePreferences } from '@/lib/queries'
import { shortcutLabel, shortcuts } from '@/lib/shortcuts'
import { useKoharuStore, type CanvasTool } from '@/lib/store'
import { commands, type Inpainter } from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'
import { Input } from '@koharu/ui/components/input'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'

/** The tools whose area the bar follows. */
const TOOLS: CanvasTool[] = ['marquee', 'lasso']

/** Generative Fill for the area the marquee and the lasso marked, in a bar below it, as
 * Photoshop's Contextual Task Bar offers it below a selection: the model it draws with, what to
 * draw there, Generate, and Deselect. The model is the fill's own, saved as
 * `[pipeline.inpainter] fill`, apart from the one that cleans pages; Generate fills the area with
 * it after the prompt, or with the inpainter's own prompt when it's empty. LaMa takes no prompt,
 * so with it the bar has none. */
export function FillBar() {
  const { t } = useTranslation()
  const area = useKoharuStore((state) => state.area)
  const tool = useKoharuStore((state) => state.tool)
  const preferences = usePreferences().data
  const catalog = useModelCatalog()
  const save = useSavePreferences()
  // Kept after Generate, to fill again with it, as Photoshop keeps it.
  const [prompt, setPrompt] = useState('')
  if (!area || !preferences || !TOOLS.includes(tool)) return null

  const pipeline = preferences.pipeline
  const model = pipeline.inpainter?.fill ?? 'flux3-klein'
  const prompted = model !== 'lama'
  const generate = () => send(commands.fill, (prompted && prompt.trim()) || null, model)
  return (
    <ContextBar bounds={area} aria-label={t('fill.title')}>
      <StageModelSelect
        size='sm'
        side='bottom'
        value={model}
        entries={catalog.data?.inpaint ?? []}
        fallbackNames={inpainters}
        availableVramBytes={catalog.data?.available_vram_bytes ?? null}
        otherStagesVramBytes={computeOtherStagesVram(catalog.data, pipeline, 'inpaint')}
        label={t('fill.model')}
        className='h-7'
        finalFocus={(closed) => closed === 'keyboard'}
        onOpen={() => void catalog.refetch()}
        onChange={(fill) =>
          save.mutate({
            ...preferences,
            pipeline: {
              ...pipeline,
              inpainter: { ...pipeline.inpainter, fill: fill as Inpainter },
            },
          })
        }
      />
      {prompted && (
        <Input
          value={prompt}
          aria-label={t('fill.prompt')}
          placeholder={t('fill.prompt')}
          className='h-7 w-60 text-[11px]'
          onChange={(event) => setPrompt(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && !event.nativeEvent.isComposing) {
              event.preventDefault()
              generate()
            }
          }}
        />
      )}
      <Button type='button' size='sm' className='h-7 text-[11px]' onClick={generate}>
        <Sparkles />
        {t('fill.generate')}
      </Button>
      <Tooltip>
        <TooltipTrigger
          render={
            <Button
              type='button'
              variant='ghost'
              size='icon-sm'
              aria-label={t('fill.deselect')}
              className='text-muted-foreground hover:text-foreground'
              onClick={() => send(commands.deselect)}
            />
          }
        >
          <X className='size-3.5' />
        </TooltipTrigger>
        <TooltipContent side='bottom'>
          {t('fill.deselect')}
          <span className='ml-2 opacity-60'>{shortcutLabel(shortcuts.deselect)}</span>
        </TooltipContent>
      </Tooltip>
    </ContextBar>
  )
}
