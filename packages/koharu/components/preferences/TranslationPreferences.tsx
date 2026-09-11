'use client'

import { ChevronDown } from 'lucide-react'
import { useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { ModelPicker } from '@/components/controls/ModelPicker'
import { GenerationPreferences } from '@/components/preferences/GenerationPreferences'
import {
  PreferencePage,
  PreferenceRow,
  PreferenceSection,
} from '@/components/preferences/PreferenceFields'
import { type EntryPoint, optionsFor, selectionInvalidated } from '@/lib/providerCapabilities'
import { modelKey, modelSelection, orderedLanguageChoices, providerName } from '@/lib/translation'
import type {
  LanguageChoice,
  Model,
  ProviderPreference,
  TranslationConfig as TranslationSettings,
} from '@koharu/bridge/protocol'
import { Badge } from '@koharu/ui/components/badge'
import { Popover, PopoverContent, PopoverTrigger } from '@koharu/ui/components/popover'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@koharu/ui/components/select'
import { Textarea } from '@koharu/ui/components/textarea'

export function TranslationPreferences({
  value,
  modelChoices,
  providers,
  languages,
  onChange,
}: {
  value: TranslationSettings
  modelChoices: Model[]
  providers: ProviderPreference[]
  languages: LanguageChoice[]
  onChange: (value: TranslationSettings) => void
}) {
  const { t } = useTranslation()
  const [modelOpen, setModelOpen] = useState(false)
  // The settings picker mirrors the translation entry point: page images are
  // uploaded when Vision is on, so the options are capability-filtered there too.
  const entryPoint: EntryPoint =
    (value.generation?.vision ?? true)
      ? { kind: 'multimodal', modality: 'image' }
      : { kind: 'text' }
  const offered = optionsFor(modelChoices, entryPoint)
  const selected =
    offered.find((candidate) => modelKey(candidate) === modelKey(value.model)) ?? null
  const current: Model = selected ?? {
    ...value.model,
    model: value.model.model ?? null,
    name: value.model.model ?? providerName(providers, value.model.provider),
    quantizations: [],
    vision: value.model.vision ?? false,
    reasoning: value.model.reasoning ?? false,
  }
  // A saved model that no longer satisfies the entry point is dropped rather
  // than silently kept, and the user is told to choose again.
  const invalidated = selectionInvalidated(modelChoices, entryPoint, value.model)
  const choices = selected || invalidated ? offered : [current, ...offered]
  const quantizations = current.quantizations
  const languageChoices = useMemo(() => orderedLanguageChoices(languages), [languages])
  return (
    <PreferencePage
      title={t('settings.translation.title')}
      description={t('settings.translation.description')}
    >
      <PreferenceSection
        title={t('settings.translation.model')}
        description={t('settings.translation.modelDescription')}
      >
        <PreferenceRow
          title={t('settings.translation.translationModel')}
          description={t('settings.translation.translationModelDescription')}
        >
          <Popover open={modelOpen} onOpenChange={setModelOpen}>
            <PopoverTrigger
              type='button'
              aria-label={t('settings.translation.translationModel')}
              className='flex h-9 w-full min-w-0 items-center justify-between gap-2 rounded-lg border border-input bg-transparent px-2.5 text-[11px] transition-colors outline-none hover:bg-foreground/[0.03] focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50'
            >
              <span className='min-w-0 flex-1 text-left'>
                <ModelLabel model={current} providers={providers} />
              </span>
              <ChevronDown className='size-3.5 shrink-0 text-muted-foreground' />
            </PopoverTrigger>
            <PopoverContent
              align='start'
              sideOffset={4}
              className='w-(--anchor-width) min-w-64 gap-0 overflow-hidden rounded-xl border border-border/50 p-1 shadow-sm ring-0'
            >
              <ModelPicker
                value={value.model}
                models={choices}
                providers={providers}
                onBack={() => setModelOpen(false)}
                onSelect={(model) => {
                  onChange({
                    ...value,
                    model: modelSelection(model),
                  })
                  setModelOpen(false)
                }}
              />
            </PopoverContent>
          </Popover>
        </PreferenceRow>
        {quantizations.length > 0 && (
          <PreferenceRow
            title={t('settings.translation.quantization')}
            description={t('settings.translation.quantizationDescription')}
          >
            <Select
              value={value.model.quantization ?? ''}
              onValueChange={(quantization) =>
                onChange({ ...value, model: { ...value.model, quantization } })
              }
            >
              <SelectTrigger
                aria-label={t('settings.translation.modelQuantization')}
                className='h-8 w-full text-[11px]'
              >
                <SelectValue placeholder={t('settings.translation.selectQuantization')} />
              </SelectTrigger>
              <SelectContent>
                {quantizations.map((quantization) => (
                  <SelectItem key={quantization.id} value={quantization.id}>
                    {quantization.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </PreferenceRow>
        )}
      </PreferenceSection>

      <GenerationPreferences
        value={value.generation}
        onChange={(generation) => onChange({ ...value, generation })}
      />

      <PreferenceSection title={t('settings.translation.output')}>
        <PreferenceRow
          title={t('model.targetLanguage')}
          description={t('settings.translation.targetLanguageDescription')}
        >
          <Select
            value={value.target_language}
            items={Object.fromEntries(
              languageChoices.map((language) => [language.tag, language.name]),
            )}
            onValueChange={(target_language) =>
              target_language && onChange({ ...value, target_language })
            }
          >
            <SelectTrigger
              aria-label={t('model.targetLanguage')}
              className='h-8 w-full text-[11px]'
            >
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
          title={t('model.instructions')}
          description={t('settings.translation.instructionsDescription')}
          align='start'
        >
          <Textarea
            aria-label={t('settings.translation.instructionsLabel')}
            value={value.instructions ?? ''}
            className='field-sizing-fixed min-h-24 resize-y overflow-y-auto text-[12px] leading-5'
            placeholder={t('settings.translation.instructionsPlaceholder')}
            onChange={(event) =>
              onChange({ ...value, instructions: event.currentTarget.value || null })
            }
          />
        </PreferenceRow>
      </PreferenceSection>
    </PreferencePage>
  )
}

function ModelLabel({ model, providers }: { model: Model; providers: ProviderPreference[] }) {
  return (
    <span className='flex min-w-0 items-center gap-2'>
      <Badge variant='outline' className='shrink-0 px-1.5 py-0 text-[9px] font-medium'>
        {providerName(providers, model.provider)}
      </Badge>
      <span className='truncate'>{model.name}</span>
    </span>
  )
}
