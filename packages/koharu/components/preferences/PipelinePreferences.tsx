'use client'

import { ChevronDown } from 'lucide-react'
import { type ReactNode, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { ModelPicker } from '@/components/controls/ModelPicker'
import { detectors, inpainters, recognizers, vendorName } from '@/components/preferences/models'
import {
  NumberField,
  PreferencePage,
  PreferenceRow,
  PreferenceSection,
} from '@/components/preferences/PreferenceFields'
import { modelName, orderedLanguageChoices, REASONING, vendorOf } from '@/lib/translation'
import type {
  Detector,
  Inpainter,
  Model,
  PipelineConfig,
  Recognizer,
  TranslatorConfig,
} from '@koharu/ipc/protocol'
import { Badge } from '@koharu/ui/components/badge'
import { Popover, PopoverContent, PopoverTrigger } from '@koharu/ui/components/popover'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@koharu/ui/components/select'
import { Switch } from '@koharu/ui/components/switch'
import { Textarea } from '@koharu/ui/components/textarea'

type StageName = 'detect' | 'recognize' | 'inpaint' | 'translate'

/** The `[pipeline]` stages in the order they run, a section each with its model first: the
 * detector with its thresholds, the recognizer, the inpainter with its prompt, and the translator
 * with its options. */
export function PipelinePreferences({
  value,
  models,
  onChange,
}: {
  value: PipelineConfig
  models: Model[]
  onChange: (value: PipelineConfig) => void
}) {
  const { t } = useTranslation()
  const detector = value.detector ?? {}
  const inpainter = value.inpainter ?? {}
  const translator = value.translator ?? {}
  const thresholds = ['text', 'onomatopoeia', 'bubble', 'panel'] as const

  return (
    <PreferencePage
      title={t('settings.pipeline.title')}
      description={t('settings.pipeline.description')}
    >
      <Stage
        stage='detect'
        model={
          <ModelSelect
            stage='detect'
            value={detector.model ?? 'koharu-layout-highres'}
            names={detectors}
            onChange={(model) => onChange({ ...value, detector: { ...detector, model } })}
          />
        }
      >
        <PreferenceRow title={t('settings.pipeline.thresholds')} align='start'>
          <div className='grid grid-cols-2 gap-2'>
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
        </PreferenceRow>
      </Stage>
      <Stage
        stage='recognize'
        model={
          <ModelSelect
            stage='recognize'
            value={value.recognizer?.model ?? 'hayai-ocr'}
            names={recognizers}
            onChange={(model: Recognizer) => onChange({ ...value, recognizer: { model } })}
          />
        }
      />
      <Stage
        stage='inpaint'
        model={
          <ModelSelect
            stage='inpaint'
            value={inpainter.model ?? 'lama'}
            names={inpainters}
            onChange={(model: Inpainter) =>
              onChange({ ...value, inpainter: { ...inpainter, model } })
            }
          />
        }
      >
        <PreferenceRow
          title={t('settings.pipeline.options.prompt')}
          description={t('settings.pipeline.options.promptDescription')}
          align='start'
        >
          <Textarea
            aria-label={t('settings.pipeline.options.prompt')}
            value={inpainter.prompt ?? ''}
            className='field-sizing-fixed min-h-28 resize-y overflow-y-auto text-[12px] leading-5'
            onChange={(event) =>
              onChange({
                ...value,
                inpainter: { ...inpainter, prompt: event.currentTarget.value },
              })
            }
          />
        </PreferenceRow>
      </Stage>
      <Stage
        stage='translate'
        model={
          <TranslatorModel
            value={translator.model ?? ''}
            models={models}
            label={t('settings.pipeline.modelLabel', {
              stage: t('settings.pipeline.stages.translate.title'),
            })}
            onChange={(model) => onChange({ ...value, translator: { ...translator, model } })}
          />
        }
      >
        <TranslatorOptions
          value={translator}
          onChange={(next) => onChange({ ...value, translator: next })}
        />
      </Stage>
    </PreferencePage>
  )
}

/** A stage's section: what it does, its model, then its options. */
function Stage({
  stage,
  model,
  children,
}: {
  stage: StageName
  model: ReactNode
  children?: ReactNode
}) {
  const { t } = useTranslation()
  return (
    <PreferenceSection
      title={t(`settings.pipeline.stages.${stage}.title`)}
      description={t(`settings.pipeline.stages.${stage}.description`)}
    >
      <PreferenceRow title={t('settings.pipeline.model')}>{model}</PreferenceRow>
      {children}
    </PreferenceSection>
  )
}

function ModelSelect<Model extends Detector | Recognizer | Inpainter>({
  stage,
  value,
  names,
  onChange,
}: {
  stage: Exclude<StageName, 'translate'>
  value: Model
  names: Record<Model, string>
  onChange: (model: Model) => void
}) {
  const { t } = useTranslation()
  const models = Object.keys(names) as Model[]
  return (
    <Select
      value={value}
      items={names}
      onValueChange={(model) => model && onChange(model as Model)}
    >
      <SelectTrigger
        aria-label={t('settings.pipeline.modelLabel', {
          stage: t(`settings.pipeline.stages.${stage}.title`),
        })}
        className='h-8 w-full text-[11px]'
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
  )
}

/** The translator's model, any provider's, labelled with where it runs. */
function TranslatorModel({
  value,
  models,
  label,
  onChange,
}: {
  value: string
  models: Model[]
  label: string
  onChange: (model: string) => void
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        type='button'
        aria-label={label}
        className='flex h-8 w-full min-w-0 items-center justify-between gap-2 rounded-lg border border-input bg-transparent px-2.5 text-[11px] transition-colors outline-none hover:bg-foreground/[0.03] focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50'
      >
        <span className='flex min-w-0 flex-1 items-center gap-2 text-left'>
          {value && (
            <Badge variant='outline' className='shrink-0 px-1.5 py-0 text-[9px] font-medium'>
              {vendorName(vendorOf(value), t)}
            </Badge>
          )}
          <span className='truncate'>{modelName(models, value) || t('inference.noModel')}</span>
        </span>
        <ChevronDown className='size-3.5 shrink-0 text-muted-foreground' />
      </PopoverTrigger>
      <PopoverContent
        align='start'
        sideOffset={4}
        className='w-(--anchor-width) min-w-64 gap-0 overflow-hidden rounded-xl border border-border/50 p-1 shadow-sm ring-0'
      >
        <ModelPicker
          value={value}
          models={models}
          onSelect={(reference) => {
            onChange(reference)
            setOpen(false)
          }}
        />
      </PopoverContent>
    </Popover>
  )
}

/** The translator's options under `[pipeline.translator]`, as rows of its section: whether the
 * model sees each page, how much it reasons, the target language, and the user's instructions,
 * which Koharu adds to its own fixed system prompt. */
function TranslatorOptions({
  value,
  onChange,
}: {
  value: TranslatorConfig
  onChange: (value: TranslatorConfig) => void
}) {
  const { t, i18n } = useTranslation()
  const languageChoices = useMemo(() => orderedLanguageChoices(i18n.language), [i18n.language])
  const reasoning = value.reasoning ?? 'default'

  return (
    <>
      <PreferenceRow
        title={t('settings.pipeline.translator.vision')}
        description={t('settings.pipeline.translator.visionDescription')}
      >
        <Switch
          aria-label={t('settings.pipeline.translator.vision')}
          checked={value.vision ?? false}
          onCheckedChange={(vision) => onChange({ ...value, vision })}
        />
      </PreferenceRow>
      <PreferenceRow
        title={t('settings.pipeline.translator.reasoning')}
        description={t('settings.pipeline.translator.reasoningDescription')}
      >
        <Select
          value={reasoning}
          items={Object.fromEntries([
            ['default', t('inference.providerDefault')],
            ...REASONING.map((level) => [level, t(`reasoning.${level}`)]),
          ])}
          onValueChange={(level) =>
            level && onChange({ ...value, reasoning: level === 'default' ? null : level })
          }
        >
          <SelectTrigger
            aria-label={t('settings.pipeline.translator.reasoning')}
            className='h-8 w-full text-[11px]'
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value='default'>{t('inference.providerDefault')}</SelectItem>
            {REASONING.map((level) => (
              <SelectItem key={level} value={level}>
                {t(`reasoning.${level}`)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </PreferenceRow>
      <PreferenceRow
        title={t('model.targetLanguage')}
        description={t('settings.pipeline.translator.targetLanguageDescription')}
      >
        <Select
          value={value.target ?? 'en'}
          items={Object.fromEntries(
            languageChoices.map((language) => [language.tag, language.name]),
          )}
          onValueChange={(target) => target && onChange({ ...value, target })}
        >
          <SelectTrigger aria-label={t('model.targetLanguage')} className='h-8 w-full text-[11px]'>
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {languageChoices.map((language) => (
              <SelectItem key={language.tag} value={language.tag}>
                {language.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </PreferenceRow>
      <PreferenceRow
        title={t('settings.pipeline.translator.instructions')}
        description={t('settings.pipeline.translator.instructionsDescription')}
        align='start'
      >
        <Textarea
          aria-label={t('settings.pipeline.translator.instructions')}
          value={value.instructions ?? ''}
          className='field-sizing-fixed min-h-28 resize-y overflow-y-auto text-[12px] leading-5'
          onChange={(event) => onChange({ ...value, instructions: event.currentTarget.value })}
        />
      </PreferenceRow>
    </>
  )
}
