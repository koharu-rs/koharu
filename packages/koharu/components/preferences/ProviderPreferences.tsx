'use client'

import { useQuery } from '@tanstack/react-query'
import { openUrl } from '@tauri-apps/plugin-opener'
import { Eraser } from 'lucide-react'
import { useId, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { vendorName } from '@/components/preferences/models'
import {
  PreferencePage,
  PreferenceRow,
  PreferenceSection,
} from '@/components/preferences/PreferenceFields'
import { call, dispatch, refreshModels } from '@/lib/backend'
import { queryClient } from '@/lib/queries'
import {
  Channel,
  commands,
  type Providers,
  type SignInPrompt,
  type Vendor,
} from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'
import { Input } from '@koharu/ui/components/input'

const vendorsKey = ['vendors'] as const

/** Lists the vendors and models again after a key or a sign-in changed. */
async function vendorsChanged() {
  await queryClient.invalidateQueries({ queryKey: vendorsKey })
  await refreshModels(true)
}

/** Each vendor's API key, or its sign-in for one the user signs in to with their plan, kept in
 * the OS credential store and saved apart from `config.toml`, and the `[providers]` base URL for a
 * vendor whose server is elsewhere. */
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
    queryFn: () => call(commands.listVendors),
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
              {vendor.signs_in ? <SignInField vendor={vendor} /> : <KeyField vendor={vendor} />}
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
function KeyField({ vendor }: { vendor: Vendor }) {
  const { t } = useTranslation()
  const inputId = useId()
  const [draft, setDraft] = useState('')
  const name = vendorName(vendor.name)

  const store = (key: string | null) =>
    void call(commands.setApiKey, vendor.name, key)
      .then(async () => {
        setDraft('')
        await vendorsChanged()
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
            vendor.connected
              ? t('settings.providers.configured')
              : t('settings.providers.notConfigured')
          }
          className='h-8 min-w-0 flex-1 text-[12px] [-webkit-text-security:disc] [&::placeholder]:[-webkit-text-security:none]'
          onChange={(event) => setDraft(event.currentTarget.value)}
          onBlur={() => draft && store(draft)}
          onKeyDown={(event) => {
            if (event.key === 'Enter') event.currentTarget.blur()
          }}
        />
        {vendor.connected && (
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

/** A vendor's sign-in with the user's plan: a button that signs in, showing the page and the code
 * the sign-in waits on, or signs out. The browser opens on the page unless the sign-in opened it. */
function SignInField({ vendor }: { vendor: Vendor }) {
  const { t } = useTranslation()
  const [waiting, setWaiting] = useState(false)
  const [prompt, setPrompt] = useState<SignInPrompt | null>(null)
  const name = vendorName(vendor.name)

  const signIn = async () => {
    setWaiting(true)
    const channel = new Channel<SignInPrompt>()
    channel.onmessage = (asked) => {
      setPrompt(asked)
      if (!asked.opened) void openUrl(asked.url).catch(() => undefined)
    }
    try {
      await call(commands.signInVendor, vendor.name, channel)
      await vendorsChanged()
    } catch {
      // `call` reports the failure.
    } finally {
      setWaiting(false)
      setPrompt(null)
    }
  }

  const signOut = () =>
    void call(commands.signOutVendor, vendor.name)
      .then(vendorsChanged)
      .catch(() => undefined)

  return (
    <div className='grid gap-1'>
      <span className='text-[10px] text-muted-foreground'>
        {t('settings.providers.subscription')}
      </span>
      {waiting ? (
        <div className='grid gap-2 rounded-md border bg-muted/40 p-2.5'>
          {prompt?.code && (
            <p className='font-mono text-base font-semibold tracking-[0.18em] select-text'>
              {prompt.code}
            </p>
          )}
          <p className='text-[11px] leading-4 text-muted-foreground'>
            {prompt === null
              ? t('settings.providers.signingIn')
              : prompt.code
                ? t('settings.providers.deviceCodeHint')
                : t('settings.providers.browserHint')}
          </p>
          <div className='flex flex-wrap gap-2'>
            {prompt && (
              <Button
                type='button'
                variant='outline'
                size='sm'
                onClick={() => void openUrl(prompt.url).catch(() => undefined)}
              >
                {t('settings.providers.openPage')}
              </Button>
            )}
            <Button
              type='button'
              variant='ghost'
              size='sm'
              onClick={() => dispatch(commands.stopSignIn)}
            >
              {t('common.cancel')}
            </Button>
          </div>
        </div>
      ) : (
        <Button
          type='button'
          variant='outline'
          size='sm'
          className='justify-self-start'
          aria-label={
            vendor.connected
              ? t('settings.providers.signOutLabel', { provider: name })
              : t('settings.providers.signInLabel', { provider: name })
          }
          onClick={vendor.connected ? signOut : () => void signIn()}
        >
          {vendor.connected ? t('settings.providers.signOut') : t('settings.providers.signIn')}
        </Button>
      )}
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
