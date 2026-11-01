'use client'

import { RefreshCw } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { call, refreshTranslationModels } from '@/lib/backend'
import { commands, type LmStudioDiscovery as DiscoveryResult } from '@koharu/bridge/protocol'
import { Button } from '@koharu/ui/components/button'

export function LmStudioDiscovery({
  baseUrl,
  onSelect,
}: {
  baseUrl: string
  onSelect: (url: string) => void
}) {
  const { t } = useTranslation()
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<DiscoveryResult | null>(null)
  const [error, setError] = useState<string | null>(null)
  let port = 1234
  let valid = true
  try {
    const url = new URL(baseUrl || 'http://localhost:1234')
    valid = url.protocol === 'http:' || url.protocol === 'https:'
    port = Number(url.port || (url.protocol === 'https:' ? 443 : 80))
  } catch {
    valid = false
  }
  const scan = async () => {
    if (busy) return
    setBusy(true)
    setError(null)
    setResult(null)
    try {
      setResult(await call(commands.discoverLmStudio, port))
      void refreshTranslationModels(true).catch(() => undefined)
    } catch (error) {
      setError(error instanceof Error ? error.message : t('settings.providers.discovery.failed'))
    } finally {
      setBusy(false)
    }
  }
  return (
    <div className='grid gap-2'>
      <Button
        type='button'
        variant='outline'
        size='sm'
        disabled={busy || !valid}
        className='justify-start text-[11px]'
        onClick={() => void scan()}
      >
        <RefreshCw className={busy ? 'size-3.5 animate-spin' : 'size-3.5'} />
        {t(busy ? 'settings.providers.discovery.scanning' : 'settings.providers.discovery.refresh')}
      </Button>
      <p className='text-[10px] text-muted-foreground' role='status' aria-live='polite'>
        {error ??
          (!valid
            ? t('settings.providers.discovery.invalidUrl')
            : result
              ? result.subnets.length === 0
                ? t('settings.providers.discovery.noNetwork')
                : result.servers.length === 0
                  ? t('settings.providers.discovery.none', { subnets: result.subnets.join(', ') })
                  : t('settings.providers.discovery.found', { count: result.servers.length })
              : t('settings.providers.discovery.help', { port }))}
      </p>
      {result?.servers.map((server) => (
        <Button
          key={server.base_url}
          type='button'
          variant='ghost'
          className='h-auto min-w-0 justify-between gap-2 px-2 py-1.5 text-[11px]'
          onClick={() => onSelect(server.base_url)}
          aria-label={t('settings.providers.discovery.use', { url: server.base_url })}
        >
          <span className='truncate'>{server.base_url}</span>
          <span className='shrink-0 text-[10px] text-muted-foreground'>
            {t('settings.providers.discovery.models', { count: server.models.length })}
          </span>
        </Button>
      ))}
    </div>
  )
}
