'use client'

import { useQuery } from '@tanstack/react-query'
import { ChevronDown } from 'lucide-react'
import { type ReactNode, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { Choice } from '@/components/controls/Choice'
import { ModelPicker } from '@/components/controls/ModelPicker'
import {
  NumberField,
  PreferencePage,
  PreferenceRow,
  PreferenceSection,
} from '@/components/preferences/PreferenceFields'
import { withEndpoint } from '@/components/preferences/ProviderPreferences'
import { detectors, inpainters, modelName, REASONING, recognizers, vendorOf } from '@/lib/models'
import { languageNames } from '@/lib/translation'
import { vendorName } from '@/lib/vendors'
import {
  commands,
  type Detector,
  type Inpainter,
  type Model,
  type PipelineConfig,
  type Providers,
  type Recognizer,
  type TranslatorConfig,
} from '@koharu/ipc/protocol'
import { Badge } from '@koharu/ui/components/badge'
import { Popover, PopoverContent, PopoverTrigger } from '@koharu/ui/components/popover'
import { Switch } from '@koharu/ui/components/switch'
import { Textarea } from '@koharu/ui/components/textarea'

const OPENROUTER = 'openrouter'

/** The choice of no pin, which no OpenRouter tag is. */
const AUTO = '/auto'

type StageName = 'detect' | 'recognize' | 'inpaint' | 'translate'

/** The `[pipeline]` stages in the order they run, a section each with its model first: the
 * detector with its thresholds, the recognizer, the inpainter with its prompt, and the translator
 * with its options, and for an OpenRouter model the provider pinned for it, which `[providers]`
 * keeps. */
export function PipelinePreferences({
  value,
  models,
  providers,
  onChange,
  onProvidersChange,
}: {
  value: PipelineConfig
  models: Model[]
  providers: Providers
  onChange: (value: PipelineConfig) => void
  onProvidersChange: (value: Providers) => void
}) {
  const { t } = useTranslation()
  const detector = value.detector ?? {}
  const inpainter = value.inpainter ?? {}
  const translator = value.translator ?? {}
  // The translator's model as OpenRouter names it, `author/slug`, when it is one of OpenRouter's.
  const openrouterModel = translator.model?.startsWith(`${OPENROUTER}/`)
    ? translator.model.slice(OPENROUTER.length + 1)
    : null
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
            className='min-h-28 resize-none text-[12px] leading-5'
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
        {openrouterModel && (
          <OpenRouterHost
            model={openrouterModel}
            value={providers[OPENROUTER]?.pins?.[openrouterModel] ?? null}
            onChange={(tag) => {
              const pins = { ...providers[OPENROUTER]?.pins }
              if (tag) pins[openrouterModel] = tag
              else delete pins[openrouterModel]
              onProvidersChange(withEndpoint(providers, OPENROUTER, { pins }))
            }}
          />
        )}
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
  return (
    <Choice
      value={value}
      choices={names}
      label={t('settings.pipeline.modelLabel', {
        stage: t(`settings.pipeline.stages.${stage}.title`),
      })}
      onChange={onChange}
    />
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
        className='w-(--anchor-width) min-w-64 gap-0 overflow-hidden p-1'
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

/** The provider an OpenRouter model's requests are pinned to, among those OpenRouter lists for
 * it, or none, to follow the routing preference. */
function OpenRouterHost({
  model,
  value,
  onChange,
}: {
  model: string
  value: string | null
  onChange: (tag: string | null) => void
}) {
  const { t } = useTranslation()
  const hosts = useQuery({
    queryKey: ['openrouter-hosts', model],
    queryFn: () => commands.listOpenrouterHosts(model),
    retry: false,
    staleTime: 5 * 60_000,
  })
  const choices: Record<string, string> = { [AUTO]: t('settings.pipeline.translator.hostAuto') }
  const labelled = (hosts.data ?? []).map((host) => {
    // What tells a host's endpoints apart: its quantization and the region or tier the rest of its
    // tag names, as `Sail Research (fp4, us)` for `sail-research/fp4/us`.
    const parts = [host.quantization, ...host.tag.split('/').slice(1)].filter(
      (part, index, all): part is string => !!part && all.indexOf(part) === index,
    )
    return [host.tag, parts.length > 0 ? `${host.name} (${parts.join(', ')})` : host.name] as const
  })
  labelled.sort(([, a], [, b]) => a.localeCompare(b))
  for (const [tag, label] of labelled) choices[tag] = label
  // A pin the list doesn't show stays shown; once the list has loaded, it is one the model no
  // longer has, which fails its requests.
  const unavailable = hosts.isSuccess && !!value && !(value in choices)
  if (value && !(value in choices)) {
    choices[value] = unavailable
      ? t('settings.pipeline.translator.hostUnavailable', { tag: value })
      : value
  }
  return (
    <PreferenceRow
      title={t('settings.pipeline.translator.host')}
      description={
        hosts.isPending
          ? t('settings.pipeline.translator.hostLoading')
          : hosts.isError
            ? t('settings.pipeline.translator.hostError')
            : unavailable
              ? t('settings.pipeline.translator.hostUnavailableDescription')
              : hosts.data.length === 0
                ? t('settings.pipeline.translator.hostNone')
                : t('settings.pipeline.translator.hostDescription')
      }
    >
      <Choice
        value={value ?? AUTO}
        choices={choices}
        label={t('settings.pipeline.translator.host')}
        onChange={(tag) => onChange(tag === AUTO ? null : tag)}
      />
    </PreferenceRow>
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
        <Textarea
          aria-label={t('settings.pipeline.translator.instructions')}
          value={value.instructions ?? ''}
          className='min-h-28 resize-none text-[12px] leading-5'
          onChange={(event) => onChange({ ...value, instructions: event.currentTarget.value })}
        />
      </PreferenceRow>
    </>
  )
}
