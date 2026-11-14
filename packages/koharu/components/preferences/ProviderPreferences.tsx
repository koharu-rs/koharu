'use client'

import { useQuery } from '@tanstack/react-query'
import { Eraser } from 'lucide-react'
import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { vendorName } from '@/components/preferences/models'
import {
  PreferencePage,
  PreferenceRow,
  PreferenceSection,
} from '@/components/preferences/PreferenceFields'
import { call, refreshModels } from '@/lib/backend'
import { queryClient } from '@/lib/queries'
import { commands, type Providers } from '@koharu/bridge/protocol'
import { Button } from '@koharu/ui/components/button'
import { Input } from '@koharu/ui/components/input'

const vendorsKey = ['vendors'] as const

/** Each vendor's API key, kept in the OS credential store and saved apart from `config.toml`,
 * and the `[providers]` base URL for a vendor whose server is elsewhere. */
export function ProviderPreferences({
  value,
  onChange,
}: {
  value: Providers
  onChange: (value: Providers) => void
}) {
  const { t } = useTranslation()
  const vendors = useQuery({
    queryKey: vendorsKey,
    queryFn: async () =>
      Object.entries(await call(commands.getVendors)).map(([name, key]) => ({ name, key })),
  })
  return (
    <PreferencePage
      title={t('settings.providers.title')}
      description={t('settings.providers.description')}
    >
      <PreferenceSection
        title={t('settings.providers.connections')}
        description={t('settings.providers.connectionsDescription')}
      >
        {(vendors.data ?? []).map((vendor) => (
          <PreferenceRow key={vendor.name} title={vendorName(vendor.name)} align='start'>
            <div className='grid gap-2'>
              <KeyField vendor={vendor} />
              <BaseUrlField
                value={value[vendor.name]?.base_url ?? ''}
                onChange={(base_url) => {
                  const next = { ...value }
                  if (base_url) next[vendor.name] = { base_url }
                  else delete next[vendor.name]
                  onChange(next)
                }}
              />
            </div>
          </PreferenceRow>
        ))}
      </PreferenceSection>
    </PreferencePage>
  )
}

/** A vendor's key, written to the credential store when the field loses the focus. */
function KeyField({ vendor }: { vendor: { name: string; key: boolean } }) {
  const { t } = useTranslation()
  const inputId = useId()
  const [draft, setDraft] = useState('')
  const name = vendorName(vendor.name)

  const store = (key: string | null) =>
    void call(commands.setApiKey, vendor.name, key)
      .then(async () => {
        setDraft('')
        await queryClient.invalidateQueries({ queryKey: vendorsKey })
        await refreshModels(true)
      })
      .catch(() => undefined)

  return (
    <div className='grid gap-1'>
      <label htmlFor={inputId} className='text-[10px] text-muted-foreground'>
        {t('settings.providers.credential')}
      </label>
      <div className='flex gap-2'>
        <Input
          id={inputId}
          aria-label={t('settings.providers.credentialLabel', { provider: name })}
          type='text'
          autoComplete='off'
          autoCapitalize='none'
          spellCheck={false}
          value={draft}
          placeholder={
            vendor.key ? t('settings.providers.configured') : t('settings.providers.notConfigured')
          }
          className='h-8 min-w-0 flex-1 text-[12px] [-webkit-text-security:disc] [&::placeholder]:[-webkit-text-security:none]'
          onChange={(event) => setDraft(event.currentTarget.value)}
          onBlur={() => draft && store(draft)}
          onKeyDown={(event) => {
            if (event.key === 'Enter') event.currentTarget.blur()
          }}
        />
        {vendor.key && (
          <Button
            type='button'
            variant='outline'
            size='icon'
            aria-label={t('settings.providers.clearCredential', { provider: name })}
            onClick={() => store(null)}
          >
            <Eraser />
          </Button>
        )}
      </div>
    </div>
  )
}

/** A base URL, which reaches the preferences when the field loses the focus holding a URL, or
 * nothing for the vendor's own server. */
function BaseUrlField({ value, onChange }: { value: string; onChange: (value: string) => void }) {
  const { t } = useTranslation()
  const [draft, setDraft] = useState(value)
  const valid = draft === '' || URL.canParse(draft)
  return (
    <label className='grid gap-1 text-[10px] text-muted-foreground'>
      {t('model.baseUrl')}
      <Input
        type='url'
        value={draft}
        aria-invalid={!valid}
        className='h-8 text-[12px] text-foreground'
        onChange={(event) => setDraft(event.currentTarget.value)}
        onBlur={() => valid && draft !== value && onChange(draft)}
      />
    </label>
  )
}
