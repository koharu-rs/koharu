'use client'

import { ChevronDown } from 'lucide-react'
import { useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { ModelPicker } from '@/components/controls/ModelPicker'
import { vendorName } from '@/components/preferences/models'
import {
  PreferencePage,
  PreferenceRow,
  PreferenceSection,
} from '@/components/preferences/PreferenceFields'
import { modelName, orderedLanguageChoices, REASONING, vendorOf } from '@/lib/translation'
import type { ModelInfo, TranslatorConfig } from '@koharu/bridge/protocol'
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

/** Whether the prompt has `{target}`, which the translator refuses one without. */
export function promptNamesTarget(value: TranslatorConfig): boolean {
  return (value.prompt ?? '').includes('{target}')
}

/** The `[pipeline.translator]` section: the model, the target language, the system prompt with
 * `{target}` where the target's name goes, whether the model sees each page, and how much it
 * reasons. */
export function TranslationPreferences({
  value,
  models,
  onChange,
}: {
  value: TranslatorConfig
  models: ModelInfo[]
  onChange: (value: TranslatorConfig) => void
}) {
  const { t, i18n } = useTranslation()
  const [modelOpen, setModelOpen] = useState(false)
  const languageChoices = useMemo(() => orderedLanguageChoices(i18n.language), [i18n.language])
  const model = value.model ?? ''
  const reasoning = value.reasoning ?? 'default'

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
              <span className='flex min-w-0 flex-1 items-center gap-2 text-left'>
                {model && (
                  <Badge variant='outline' className='shrink-0 px-1.5 py-0 text-[9px] font-medium'>
                    {vendorName(vendorOf(model))}
                  </Badge>
                )}
                <span className='truncate'>
                  {modelName(models, model) || t('inference.noModel')}
                </span>
              </span>
              <ChevronDown className='size-3.5 shrink-0 text-muted-foreground' />
            </PopoverTrigger>
            <PopoverContent
              align='start'
              sideOffset={4}
              className='w-(--anchor-width) min-w-64 gap-0 overflow-hidden rounded-xl border border-border/50 p-1 shadow-sm ring-0'
            >
              <ModelPicker
                value={model}
                models={models}
                onBack={() => setModelOpen(false)}
                onSelect={(reference) => {
                  onChange({ ...value, model: reference })
                  setModelOpen(false)
                }}
              />
            </PopoverContent>
          </Popover>
        </PreferenceRow>
        <PreferenceRow
          title={t('settings.translation.vision')}
          description={t('settings.translation.visionDescription')}
        >
          <Switch
            aria-label={t('settings.translation.vision')}
            checked={value.vision ?? false}
            onCheckedChange={(vision) => onChange({ ...value, vision })}
          />
        </PreferenceRow>
        <PreferenceRow
          title={t('settings.translation.reasoning')}
          description={t('settings.translation.reasoningDescription')}
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
              aria-label={t('settings.translation.reasoning')}
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
      </PreferenceSection>

      <PreferenceSection title={t('settings.translation.output')}>
        <PreferenceRow
          title={t('model.targetLanguage')}
          description={t('settings.translation.targetLanguageDescription')}
        >
          <Select
            value={value.target ?? 'en'}
            items={Object.fromEntries(
              languageChoices.map((language) => [language.tag, language.name]),
            )}
            onValueChange={(target) => target && onChange({ ...value, target })}
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
          title={t('settings.translation.prompt')}
          description={t('settings.translation.promptDescription')}
          align='start'
        >
          <div className='grid gap-1'>
            <Textarea
              aria-label={t('settings.translation.prompt')}
              aria-invalid={!promptNamesTarget(value)}
              value={value.prompt ?? ''}
              className='field-sizing-fixed min-h-32 resize-y overflow-y-auto text-[12px] leading-5'
              onChange={(event) => onChange({ ...value, prompt: event.currentTarget.value })}
            />
            {!promptNamesTarget(value) && (
              <p className='text-[10px] text-destructive'>
                {t('settings.translation.promptNeedsTarget')}
              </p>
            )}
          </div>
        </PreferenceRow>
      </PreferenceSection>
    </PreferencePage>
  )
}
