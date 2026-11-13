'use client'

import { Eraser, FileText, Search } from 'lucide-react'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { detectors, inpainters, recognizers, takesPrompt } from '@/components/preferences/models'
import {
  NumberField,
  PreferencePage,
  PreferenceRow,
  PreferenceSection,
  TextField,
} from '@/components/preferences/PreferenceFields'
import type { Detector, Inpainter, PipelineConfig, Recognizer } from '@koharu/bridge/protocol'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@koharu/ui/components/select'

/** The `[pipeline]` models: the detector with its thresholds, the recognizer, and the inpainter
 * with its prompt. The translator has its own page. */
export function PipelinePreferences({
  value,
  onChange,
}: {
  value: PipelineConfig
  onChange: (value: PipelineConfig) => void
}) {
  const { t } = useTranslation()
  const detector = value.detector ?? {}
  const inpainter = value.inpainter ?? {}
  const thresholds = ['text', 'onomatopoeia', 'bubble', 'panel'] as const

  return (
    <PreferencePage
      title={t('settings.pipeline.title')}
      description={t('settings.pipeline.description')}
    >
      <PreferenceSection title={t('settings.pipeline.processing')}>
        <Stage
          stage='detect'
          icon={<Search className='size-3.5 shrink-0 text-muted-foreground' />}
          value={detector.model ?? 'koharu-layout-highres'}
          names={detectors}
          onChange={(model) => onChange({ ...value, detector: { ...detector, model } })}
        >
          <div className='grid grid-cols-4 gap-2'>
            {thresholds.map((threshold) => (
              <NumberField
                key={threshold}
                label={t(`settings.pipeline.options.${threshold}Threshold`)}
                value={detector[threshold] ?? null}
                min={0}
                max={1}
                step={0.05}
                onChange={(next) =>
                  onChange({ ...value, detector: { ...detector, [threshold]: next } })
                }
              />
            ))}
          </div>
        </Stage>
        <Stage
          stage='recognize'
          icon={<FileText className='size-3.5 shrink-0 text-muted-foreground' />}
          value={value.recognizer?.model ?? 'manga-ocr-ar'}
          names={recognizers}
          onChange={(model: Recognizer) => onChange({ ...value, recognizer: { model } })}
        />
        <Stage
          stage='inpaint'
          icon={<Eraser className='size-3.5 shrink-0 text-muted-foreground' />}
          value={inpainter.model ?? 'lama'}
          names={inpainters}
          onChange={(model: Inpainter) => onChange({ ...value, inpainter: { ...inpainter, model } })}
        >
          {takesPrompt(inpainter.model ?? 'lama') && (
            <TextField
              label={t('settings.pipeline.options.prompt')}
              value={inpainter.prompt ?? ''}
              onChange={(prompt) => onChange({ ...value, inpainter: { ...inpainter, prompt } })}
            />
          )}
        </Stage>
      </PreferenceSection>
    </PreferencePage>
  )
}

function Stage<Model extends Detector | Recognizer | Inpainter>({
  stage,
  icon,
  value,
  names,
  onChange,
  children,
}: {
  stage: 'detect' | 'recognize' | 'inpaint'
  icon: ReactNode
  value: Model
  names: Record<Model, string>
  onChange: (model: Model) => void
  children?: ReactNode
}) {
  const { t } = useTranslation()
  const title = t(`settings.pipeline.stages.${stage}.title`)
  const models = Object.keys(names) as Model[]
  return (
    <PreferenceRow
      title={title}
      description={t(`settings.pipeline.stages.${stage}.description`)}
      align='start'
    >
      <div className='grid gap-3'>
        <div className='flex items-center gap-2'>
          {icon}
          <Select
            value={value}
            items={names}
            onValueChange={(model) => model && onChange(model as Model)}
          >
            <SelectTrigger
              aria-label={t('settings.pipeline.modelLabel', { stage: title })}
              className='h-8 min-w-0 flex-1 text-[11px]'
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {models.map((model) => (
                <SelectItem key={model} value={model}>
                  {names[model]}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        {children}
      </div>
    </PreferenceRow>
  )
}
