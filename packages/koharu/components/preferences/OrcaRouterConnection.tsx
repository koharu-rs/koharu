'use client'

import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { commands } from '@koharu/bridge/protocol'
import { Button } from '@koharu/ui/components/button'

/**
 * The browser authorization half of the OrcaRouter connection.
 *
 * The API-key field beside it stays an independent choice: a user who already
 * holds an `sk-orca-…` key never has to start a login, and a user without one
 * never has to paste anything.
 */
export function OrcaRouterAuthorization({
  connect = () => commands.connectOrcaRouter(),
  cancel = () => commands.cancelOrcaRouter(),
  needsReauth = false,
  onConnected,
}: {
  connect?: () => Promise<{ authorization_url?: string } | undefined>
  cancel?: () => Promise<unknown>
  needsReauth?: boolean
  onConnected?: () => void
}) {
  const { t } = useTranslation()
  const [busy, setBusy] = useState(false)
  const [authorizationUrl, setAuthorizationUrl] = useState<string | null>(null)
  const [failed, setFailed] = useState(false)
  // Monotonic attempt id: a late response from an earlier attempt must not
  // overwrite the state of the one that replaced it.
  const attempt = useRef(0)

  useEffect(() => {
    // Back-forward cache: the page can be restored without remounting, so busy
    // state and the hint are cleared here rather than in the guarded `finally`,
    // which correctly refuses to touch invalidated state.
    const onPageHide = () => {
      attempt.current += 1
      setBusy(false)
      setAuthorizationUrl(null)
      void cancel().catch(() => undefined)
    }
    window.addEventListener('pagehide', onPageHide)
    return () => window.removeEventListener('pagehide', onPageHide)
  }, [cancel])

  const start = () => {
    const current = (attempt.current += 1)
    setBusy(true)
    setFailed(false)
    setAuthorizationUrl(null)
    void connect()
      .then((result) => {
        if (attempt.current !== current) return
        setAuthorizationUrl(result?.authorization_url ?? null)
        onConnected?.()
      })
      .catch(() => {
        if (attempt.current !== current) return
        setFailed(true)
      })
      .finally(() => {
        if (attempt.current !== current) return
        setBusy(false)
      })
  }

  const stop = () => {
    attempt.current += 1
    setBusy(false)
    setAuthorizationUrl(null)
    void cancel().catch(() => undefined)
  }

  return (
    <div className='grid gap-1 border-t border-border/60 pt-2'>
      <span className='text-[10px] text-muted-foreground'>
        {t('settings.providers.authMethod')}
      </span>
      <p className='text-[10px] leading-4 text-muted-foreground'>
        {t('settings.providers.connectDescription')}
      </p>
      <div className='flex gap-2'>
        <Button
          type='button'
          variant='outline'
          size='sm'
          className='h-8'
          disabled={busy}
          aria-busy={busy}
          aria-label={t('settings.providers.connect')}
          onClick={start}
        >
          {busy ? t('settings.providers.connecting') : t('settings.providers.connect')}
        </Button>
        {busy && (
          <Button
            type='button'
            variant='ghost'
            size='sm'
            className='h-8'
            aria-label={t('settings.providers.cancelAuthorization')}
            onClick={stop}
          >
            {t('settings.providers.cancelAuthorization')}
          </Button>
        )}
      </div>
      {authorizationUrl && (
        <a
          className='truncate text-[10px] text-muted-foreground underline'
          href={authorizationUrl}
          target='_blank'
          rel='noreferrer'
        >
          {authorizationUrl}
        </a>
      )}
      {(failed || needsReauth) && (
        <p role='alert' className='text-[10px] text-destructive'>
          {failed ? t('settings.providers.connectFailed') : t('settings.providers.reauth')}
        </p>
      )}
    </div>
  )
}
