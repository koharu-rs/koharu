'use client'

import { useMutation, useQuery } from '@tanstack/react-query'
import { openUrl } from '@tauri-apps/plugin-opener'
import { Eraser } from 'lucide-react'
import { useEffect, useId, useState } from 'react'
import { useTranslation } from 'react-i18next'

import {
  NumberField,
  PreferencePage,
  PreferenceRow,
  PreferenceSection,
} from '@/components/preferences/PreferenceFields'
import { modelsKey, refresh, send } from '@/lib/queries'
import { vendorName, vendorTraits } from '@/lib/vendors'
import {
  Channel,
  commands,
  type Endpoint,
  type Providers,
  type SignInPrompt,
  type Vendor,
} from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'
import { Input } from '@koharu/ui/components/input'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'

const vendorsKey = ['vendors'] as const

/** Lists the vendors and models again after a key or a sign-in changed. */
function vendorsChanged() {
  return refresh(vendorsKey, modelsKey)
}

/** Each vendor's API key, or its sign-in for one the user signs in to with their plan, kept in
 * the OS credential store and saved apart from `config.toml`, and the `[providers]` base URL for a
 * vendor whose server the user names, with the context window of a server that doesn't say. */
export function ProviderPreferences({
  value,
  onChange,
}: {
  value: Providers
  onChange: (value: Providers) => void
}) {
  const { t } = useTranslation()
  const vendors = useQuery({ queryKey: vendorsKey, queryFn: () => commands.listVendors() })
  /** `vendor`'s endpoint with `patch` merged in; an endpoint with nothing left set goes. */
  const update = (vendor: string, patch: Endpoint) => {
    const endpoint: Endpoint = { ...value[vendor], ...patch }
    if (!endpoint.base_url) delete endpoint.base_url
    if (!endpoint.context_window) delete endpoint.context_window
    const next = { ...value }
    if (Object.keys(endpoint).length > 0) next[vendor] = endpoint
    else delete next[vendor]
    onChange(next)
  }
  return (
    <PreferencePage
      title={t('settings.providers.title')}
      description={t('settings.providers.description')}
    >
      <PreferenceSection
        title={t('settings.providers.connections')}
        description={t('settings.providers.connectionsDescription')}
      >
        {(vendors.data ?? []).map((vendor) => {
          const traits = vendorTraits[vendor.name] ?? {}
          return (
            <PreferenceRow key={vendor.name} title={vendorName(vendor.name, t)} align='start'>
              <div className='grid gap-2'>
                {traits.signsIn ? <SignInField vendor={vendor} /> : <KeyField vendor={vendor} />}
                {traits.server && (
                  <BaseUrlField
                    value={value[vendor.name]?.base_url ?? ''}
                    onChange={(base_url) => update(vendor.name, { base_url })}
                  />
                )}
                {/* The context window, in tokens, the server runs its models with, such as
                    llama.cpp's `--ctx-size`; nothing while the server's is unknown. */}
                {traits.contextWindow && (
                  <NumberField
                    label={t('settings.providers.contextWindow')}
                    value={value[vendor.name]?.context_window ?? null}
                    min={1}
                    max={2 ** 32 - 1}
                    step={1}
                    integer
                    onChange={(context_window) => update(vendor.name, { context_window })}
                  />
                )}
              </div>
            </PreferenceRow>
          )
        })}
      </PreferenceSection>
    </PreferencePage>
  )
}

/** A vendor's key, written to the credential store when the field loses the focus. */
function KeyField({ vendor }: { vendor: Vendor }) {
  const { t } = useTranslation()
  const inputId = useId()
  const [draft, setDraft] = useState('')
  const name = vendorName(vendor.name, t)

  const store = useMutation({
    mutationFn: (key: string | null) => commands.setApiKey(vendor.name, key),
    onSuccess: async () => {
      setDraft('')
      await vendorsChanged()
    },
  })

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
          onBlur={() => draft && store.mutate(draft)}
          onKeyDown={(event) => {
            if (event.key === 'Enter') event.currentTarget.blur()
          }}
        />
        {vendor.connected && (
          <Tooltip>
            <TooltipTrigger
              render={
                <Button
                  type='button'
                  variant='destructive'
                  size='icon'
                  aria-label={t('settings.providers.clearCredential', { provider: name })}
                  onClick={() => store.mutate(null)}
                />
              }
            >
              <Eraser />
            </TooltipTrigger>
            <TooltipContent>
              {t('settings.providers.clearCredential', { provider: name })}
            </TooltipContent>
          </Tooltip>
        )}
      </div>
    </div>
  )
}

/** A vendor's sign-in with the user's plan: a button that signs in, showing the page and the code
 * the sign-in waits on, or signs out. The browser opens on the page unless the sign-in opened it. */
function SignInField({ vendor }: { vendor: Vendor }) {
  const { t } = useTranslation()
  const [prompt, setPrompt] = useState<SignInPrompt | null>(null)
  const name = vendorName(vendor.name, t)
  const signIn = useMutation({
    mutationFn: () => {
      const channel = new Channel<SignInPrompt>()
      channel.onmessage = (asked) => {
        setPrompt(asked)
        if (!asked.opened) void openUrl(asked.url).catch(() => undefined)
      }
      return commands.signIn(vendor.name, channel)
    },
    onSuccess: vendorsChanged,
    onSettled: () => setPrompt(null),
  })
  const signOut = useMutation({
    mutationFn: () => commands.signOut(vendor.name),
    onSuccess: vendorsChanged,
  })

  return (
    <div className='grid gap-1'>
      <span className='text-[10px] text-muted-foreground'>
        {t('settings.providers.subscription')}
      </span>
      {signIn.isPending ? (
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
              onClick={() => send(commands.stopSignIn)}
            >
              {t('common.cancel')}
            </Button>
          </div>
        </div>
      ) : (
        <Button
          type='button'
          variant={vendor.connected ? 'destructive' : 'outline'}
          size='sm'
          className='justify-self-start'
          aria-label={
            vendor.connected
              ? t('settings.providers.signOutLabel', { provider: name })
              : t('settings.providers.signInLabel', { provider: name })
          }
          onClick={() => (vendor.connected ? signOut.mutate() : signIn.mutate())}
        >
          {vendor.connected ? t('settings.providers.signOut') : t('settings.providers.signIn')}
        </Button>
      )}
    </div>
  )
}

/** A base URL, which reaches the preferences when the field loses the focus holding a URL, or
 * nothing for the vendor's own server, and shows what they then hold. */
function BaseUrlField({ value, onChange }: { value: string; onChange: (value: string) => void }) {
  const { t } = useTranslation()
  const [draft, setDraft] = useState(value)

  useEffect(() => {
    setDraft(value)
  }, [value])

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
