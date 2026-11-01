'use client'

import { Eraser } from 'lucide-react'
import { useEffect, useId, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { LmStudioDiscovery } from '@/components/preferences/LmStudioDiscovery'
import {
  PreferencePage,
  PreferenceRow,
  PreferenceSection,
  TextField,
} from '@/components/preferences/PreferenceFields'
import type {
  CredentialInput,
  ProviderConfig,
  ProviderPreference,
  ProviderPreferences as ProviderSettings,
} from '@koharu/bridge/protocol'
import { Button } from '@koharu/ui/components/button'
import { Input } from '@koharu/ui/components/input'

type ConfigWithSetting<T, Key extends PropertyKey> = T extends { settings: infer Settings }
  ? Key extends keyof Settings
    ? [Settings[Key]] extends [never]
      ? never
      : T
    : never
  : never

type BaseUrlConfig = ConfigWithSetting<ProviderConfig, 'base_url'>

export function ProviderPreferences({
  value,
  onChange,
}: {
  value: ProviderSettings
  onChange: (value: ProviderSettings) => void
}) {
  const { t } = useTranslation()
  return (
    <PreferencePage
      title={t('settings.providers.title')}
      description={t('settings.providers.description')}
    >
      <PreferenceSection
        title={t('settings.providers.connections')}
        description={t('settings.providers.connectionsDescription')}
      >
        {value.entries.filter(isConfigurable).map((entry) => (
          <PreferenceRow
            key={entry.config.provider}
            title={entry.name}
            description={t(`providerDescriptions.${entry.config.provider}`)}
            align='start'
          >
            <div className='grid gap-2'>
              {entry.credential && (
                <CredentialField
                  label={entry.name}
                  value={entry.credential}
                  onChange={(credential) => onChange(replaceEntry(value, { ...entry, credential }))}
                />
              )}
              {hasBaseUrl(entry.config) && (
                <TextField
                  label={t('model.baseUrl')}
                  type='url'
                  value={entry.config.settings.base_url ?? ''}
                  onChange={(base_url) =>
                    onChange(
                      replaceEntry(value, {
                        ...entry,
                        config: withBaseUrl(entry.config, base_url),
                      }),
                    )
                  }
                />
              )}
              {entry.config.provider === 'lm-studio' && (
                <LmStudioDiscovery
                  baseUrl={entry.config.settings.base_url ?? ''}
                  onSelect={(base_url) =>
                    onChange(
                      replaceEntry(value, {
                        ...entry,
                        config: withBaseUrl(entry.config, base_url),
                      }),
                    )
                  }
                />
              )}
            </div>
          </PreferenceRow>
        ))}
      </PreferenceSection>
    </PreferencePage>
  )
}

function CredentialField({
  label,
  value,
  onChange,
}: {
  label: string
  value: CredentialInput
  onChange: (value: CredentialInput) => void
}) {
  const { t } = useTranslation()
  const inputId = useId()
  const [draftValue, setDraftValue] = useState(value.value ?? '')
  useEffect(() => {
    if (value.value !== null) setDraftValue(value.value)
    else if (!value.configured || value.clear) setDraftValue('')
  }, [value.clear, value.configured, value.value])
  const configured = !value.clear && (value.configured || Boolean(draftValue))
  return (
    <div className='grid gap-1'>
      <label htmlFor={inputId} className='text-[10px] text-muted-foreground'>
        {t('settings.providers.credential')}
      </label>
      <div className='flex gap-2'>
        <Input
          id={inputId}
          aria-label={t('settings.providers.credentialLabel', { provider: label })}
          type='text'
          autoComplete='off'
          autoCapitalize='none'
          spellCheck={false}
          value={draftValue}
          placeholder={
            configured ? t('settings.providers.configured') : t('settings.providers.notConfigured')
          }
          className='h-8 min-w-0 flex-1 text-[12px] [-webkit-text-security:disc] [&::placeholder]:[-webkit-text-security:none]'
          onChange={(event) => {
            const draft = event.currentTarget.value
            setDraftValue(draft)
            onChange({ ...value, value: draft || null, clear: false })
          }}
        />
        {configured && (
          <Button
            type='button'
            variant='outline'
            size='icon'
            aria-label={t('settings.providers.clearCredential', { provider: label })}
            onClick={() => {
              setDraftValue('')
              onChange({ configured: false, value: null, clear: true })
            }}
          >
            <Eraser />
          </Button>
        )}
      </div>
    </div>
  )
}

function isConfigurable(entry: ProviderPreference): boolean {
  return entry.credential !== null || hasBaseUrl(entry.config)
}

function hasBaseUrl(config: ProviderConfig): config is BaseUrlConfig {
  return 'base_url' in config.settings
}

function withBaseUrl(config: ProviderConfig, base_url: string): ProviderConfig {
  if (!hasBaseUrl(config)) return config
  return {
    ...config,
    settings: { ...config.settings, base_url: base_url || null },
  } as ProviderConfig
}

function replaceEntry(
  preferences: ProviderSettings,
  replacement: ProviderPreference,
): ProviderSettings {
  return {
    entries: preferences.entries.map((entry) =>
      entry.config.provider === replacement.config.provider ? replacement : entry,
    ),
  }
}
