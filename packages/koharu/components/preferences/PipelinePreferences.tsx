'use client'

import { ChevronDown } from 'lucide-react'
import { type ReactNode, useEffect, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Choice } from '@/components/controls/Choice'
import {
  ModelDiskMeta,
  ModelPicker,
  StageModelSelect,
  VramFitBadge,
} from '@/components/controls/ModelPicker'
import {
  NumberField,
  PreferencePage,
  PreferenceRow,
  PreferenceSection,
} from '@/components/preferences/PreferenceFields'
import {
  computeOtherStagesVram,
  detectors,
  findCatalogSelection,
  formatModelName,
  inpainters,
  REASONING,
  recognizers,
  vendorOf,
} from '@/lib/models'
import { languageNames } from '@/lib/translation'
import { vendorName } from '@/lib/vendors'
import type {
  Detector,
  Inpainter,
  ModelCatalog,
  PipelineConfig,
  Recognizer,
  Stage as StageEnum,
  TranslateModelEntry,
  TranslatorConfig,
} from '@koharu/ipc/protocol'
import { Badge } from '@koharu/ui/components/badge'
import {
  NumberField as NumberFieldRoot,
  NumberFieldGroup,
  NumberFieldInput,
} from '@koharu/ui/components/number-field'
import { Popover, PopoverContent, PopoverTrigger } from '@koharu/ui/components/popover'
import { Switch } from '@koharu/ui/components/switch'
import { Textarea } from '@koharu/ui/components/textarea'

/** The `[pipeline]` stages in the order they run, a section each with its model first: the
 * detector with its thresholds, the recognizer, the inpainter with its prompt, and the translator
 * with its options. */
export function PipelinePreferences({
  value,
  catalog,
  onRefreshCatalog,
  onChange,
}: {
  value: PipelineConfig
  catalog: ModelCatalog | undefined
  onRefreshCatalog: () => void
  onChange: (value: PipelineConfig) => void
}) {
  const { t } = useTranslation()
  const detector = value.detector ?? {}
  const inpainter = value.inpainter ?? {}
  const translator = value.translator ?? {}
  const thresholds = ['text', 'onomatopoeia', 'bubble', 'panel'] as const
  const availableVramBytes = catalog?.available_vram_bytes ?? null

  return (
    <PreferencePage
      title={t('settings.pipeline.title')}
      description={t('settings.pipeline.description')}
    >
      <Stage
        stage='detect'
        model={
          <StageModelSelect
            value={detector.model ?? 'koharu-layout-highres'}
            entries={catalog?.detect ?? []}
            fallbackNames={detectors}
            availableVramBytes={availableVramBytes}
            otherStagesVramBytes={computeOtherStagesVram(catalog, value, 'detect')}
            label={t('settings.pipeline.modelLabel', {
              stage: t('settings.pipeline.stages.detect.title'),
            })}
            onOpen={onRefreshCatalog}
            onChange={(model) =>
              onChange({ ...value, detector: { ...detector, model: model as Detector } })
            }
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
                placeholder={t('model.default')}
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
          <StageModelSelect
            value={value.recognizer?.model ?? 'hayai-ocr'}
            entries={catalog?.recognize ?? []}
            fallbackNames={recognizers}
            availableVramBytes={availableVramBytes}
            otherStagesVramBytes={computeOtherStagesVram(catalog, value, 'recognize')}
            label={t('settings.pipeline.modelLabel', {
              stage: t('settings.pipeline.stages.recognize.title'),
            })}
            onOpen={onRefreshCatalog}
            onChange={(model) => onChange({ ...value, recognizer: { model: model as Recognizer } })}
          />
        }
      />
      <Stage
        stage='inpaint'
        model={
          <StageModelSelect
            value={inpainter.model ?? 'lama'}
            entries={catalog?.inpaint ?? []}
            fallbackNames={inpainters}
            availableVramBytes={availableVramBytes}
            otherStagesVramBytes={computeOtherStagesVram(catalog, value, 'inpaint')}
            label={t('settings.pipeline.modelLabel', {
              stage: t('settings.pipeline.stages.inpaint.title'),
            })}
            onOpen={onRefreshCatalog}
            onChange={(model) =>
              onChange({ ...value, inpainter: { ...inpainter, model: model as Inpainter } })
            }
          />
        }
      >
        <PreferenceRow
          title={t('settings.pipeline.options.prompt')}
          description={t('settings.pipeline.options.promptDescription')}
          align='start'
        >
          <TextBox
            label={t('settings.pipeline.options.prompt')}
            value={inpainter.prompt ?? ''}
            onChange={(prompt) => onChange({ ...value, inpainter: { ...inpainter, prompt } })}
          />
        </PreferenceRow>
      </Stage>
      <Stage
        stage='translate'
        model={
          <TranslatorModel
            value={translator.model ?? ''}
            models={catalog?.translate ?? []}
            availableVramBytes={availableVramBytes}
            otherStagesVramBytes={computeOtherStagesVram(catalog, value, 'translate')}
            label={t('settings.pipeline.modelLabel', {
              stage: t('settings.pipeline.stages.translate.title'),
            })}
            onOpen={onRefreshCatalog}
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
  stage: StageEnum
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

/** The translator's model, any provider's, labelled with where it runs and its footprint. */
function TranslatorModel({
  value,
  models,
  availableVramBytes,
  otherStagesVramBytes,
  label,
  onOpen,
  onChange,
}: {
  value: string
  models: TranslateModelEntry[]
  availableVramBytes: number | null
  otherStagesVramBytes: number
  label: string
  onOpen: () => void
  onChange: (model: string) => void
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const selection = useMemo(() => findCatalogSelection(models, value), [models, value])
  const diskBytes = selection?.variant?.disk_bytes ?? selection?.model.disk_bytes ?? null
  const downloaded = selection?.variant?.downloaded ?? selection?.model.downloaded ?? false
  const measuredVram =
    selection?.variant?.measured_vram_bytes ?? selection?.model.measured_vram_bytes ?? null

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next)
        if (next) onOpen()
      }}
    >
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
          <span className='min-w-0 truncate'>
            {formatModelName(models, value, t) || t('inference.noModel')}
          </span>
          {diskBytes !== null && <ModelDiskMeta diskBytes={diskBytes} downloaded={downloaded} />}
          <VramFitBadge
            candidateVramBytes={measuredVram}
            otherStagesVramBytes={otherStagesVramBytes}
            availableVramBytes={availableVramBytes}
          />
        </span>
        <ChevronDown className='size-3.5 shrink-0 text-muted-foreground' />
      </PopoverTrigger>
      <PopoverContent
        align='start'
        sideOffset={4}
        className='w-(--anchor-width) min-w-64 gap-0 overflow-hidden p-1'
      >
        <ModelPicker
          value={value}
          models={models}
          availableVramBytes={availableVramBytes}
          otherStagesVramBytes={otherStagesVramBytes}
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
  const languageChoices = useMemo(() => languageNames(i18n.language), [i18n.language])
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
        title={t('settings.pipeline.translator.combinePages')}
        description={t('settings.pipeline.translator.combinePagesDescription')}
      >
        <Switch
          aria-label={t('settings.pipeline.translator.combinePages')}
          checked={value.combine_pages ?? true}
          onCheckedChange={(combine_pages) => onChange({ ...value, combine_pages })}
        />
      </PreferenceRow>
      {(value.combine_pages ?? true) && (
        <PreferenceRow
          title={t('settings.pipeline.translator.maxPages')}
          description={t('settings.pipeline.translator.maxPagesDescription')}
        >
          <NumberFieldRoot
            value={value.max_pages_per_request ?? null}
            min={1}
            max={64}
            step={1}
            format={{ maximumFractionDigits: 0, useGrouping: false }}
            onValueChange={(next) =>
              onChange({
                ...value,
                max_pages_per_request: next === null ? null : Math.max(1, Math.round(next)),
              })
            }
          >
            <NumberFieldGroup className='h-8 rounded-lg focus-within:ring-3 focus-within:ring-ring/50'>
              <NumberFieldInput
                aria-label={t('settings.pipeline.translator.maxPages')}
                placeholder={t('settings.pipeline.translator.maxPagesAuto')}
                className='px-2.5 text-left text-[12px] text-foreground placeholder:text-muted-foreground'
              />
            </NumberFieldGroup>
          </NumberFieldRoot>
        </PreferenceRow>
      )}
      <PreferenceRow
        title={t('settings.pipeline.translator.reasoning')}
        description={t('settings.pipeline.translator.reasoningDescription')}
      >
        <Choice
          value={reasoning}
          choices={Object.fromEntries([
            ['default', t('inference.providerDefault')],
            ...REASONING.map((level) => [level, t(`reasoning.${level}`)]),
          ])}
          label={t('settings.pipeline.translator.reasoning')}
          onChange={(level) =>
            onChange({ ...value, reasoning: level === 'default' ? null : level })
          }
        />
      </PreferenceRow>
      <PreferenceRow
        title={t('model.targetLanguage')}
        description={t('settings.pipeline.translator.targetLanguageDescription')}
      >
        <Choice
          value={value.target ?? 'en'}
          choices={languageChoices}
          label={t('model.targetLanguage')}
          onChange={(target) => onChange({ ...value, target })}
        />
      </PreferenceRow>
      <PreferenceRow
        title={t('settings.pipeline.translator.instructions')}
        description={t('settings.pipeline.translator.instructionsDescription')}
        align='start'
      >
        <TextBox
          label={t('settings.pipeline.translator.instructions')}
          value={value.instructions ?? ''}
          onChange={(instructions) => onChange({ ...value, instructions })}
        />
      </PreferenceRow>
    </>
  )
}

/** A text box that holds what's typed itself. The preferences show an edit a tick later, and a box
 * waiting for them would put the older text back after each keystroke, ending the input method's
 * composition. */
function TextBox({
  label,
  value,
  onChange,
}: {
  label: string
  value: string
  onChange: (value: string) => void
}) {
  const [draft, setDraft] = useState(value)

  useEffect(() => {
    setDraft(value)
  }, [value])

  return (
    <Textarea
      aria-label={label}
      value={draft}
      className='min-h-28 resize-none text-[12px] leading-5'
      onChange={(event) => {
        setDraft(event.currentTarget.value)
        onChange(event.currentTarget.value)
      }}
    />
  )
}
