'use client'

import { Channel } from '@tauri-apps/api/core'
import {
  ArrowLeft,
  ArrowRight,
  BookOpen,
  Home,
  Languages,
  LoaderCircle,
  RefreshCw,
  Trash2,
} from 'lucide-react'
import { useEffect, useRef, useState, type FormEvent } from 'react'
import { useTranslation } from 'react-i18next'

import { call, savePreferences } from '@/lib/backend'
import { pageKey, pagesKey, projectKey, refresh } from '@/lib/queries'
import { receivePreferences, useKoharuStore } from '@/lib/store'
import { orderedLanguageChoices } from '@/lib/translation'
import { commands, type SourcePage, type SourceProgress } from '@koharu/bridge/protocol'
import { Button } from '@koharu/ui/components/button'
import { Input } from '@koharu/ui/components/input'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@koharu/ui/components/select'

const home = 'https://rawkuma.net/'

export function SourceBrowser() {
  const { t } = useTranslation()
  const container = useRef<HTMLDivElement>(null)
  const [page, setPage] = useState<SourcePage | null>(null)
  const [address, setAddress] = useState(home)
  const [progress, setProgress] = useState<SourceProgress | null>(null)
  const [revalidate, setRevalidate] = useState(true)
  const [replace, setReplace] = useState(false)
  const [message, setMessage] = useState<string | null>(null)
  const [pageError, setPageError] = useState<string | null>(null)
  const busy = useKoharuStore((state) => state.sourceBusy)
  const jobs = useKoharuStore((state) => state.jobs)
  const processing = Object.values(jobs).some((job) => job.state === 'running')
  const preferences = useKoharuStore((state) => state.preferences)
  const languages = orderedLanguageChoices(preferences?.languages ?? [])
  const setBusy = useKoharuStore((state) => state.setSourceBusy)
  const setOpen = useKoharuStore((state) => state.setSourceOpen)

  useEffect(() => {
    let disposed = false
    let ready = false
    let polling = false
    let frame = 0
    let lastUrl = home
    const bounds = () => {
      const rect = container.current!.getBoundingClientRect()
      return { x: rect.x, y: rect.y, width: rect.width, height: rect.height }
    }
    const inspect = async () => {
      if (!ready || disposed || polling) return
      polling = true
      try {
        const next = await commands.inspectSourcePage()
        if (!disposed) {
          setPage(next)
          setPageError(null)
          const previousUrl = lastUrl
          setAddress((current) => (current === previousUrl ? next.url : current))
          lastUrl = next.url
        }
      } catch (error) {
        if (!disposed && error instanceof Error) setPageError(error.message)
      } finally {
        polling = false
      }
    }
    void call(commands.openSourceBrowser, bounds())
      .then(async () => {
        if (disposed) return
        ready = true
        await inspect()
      })
      .catch(() => undefined)
    const observer = new ResizeObserver(() => {
      cancelAnimationFrame(frame)
      frame = requestAnimationFrame(() => {
        if (ready && !disposed) void commands.resizeSourceBrowser(bounds()).catch(() => undefined)
      })
    })
    observer.observe(container.current!)
    const interval = setInterval(() => void inspect(), 2000)
    return () => {
      disposed = true
      clearInterval(interval)
      cancelAnimationFrame(frame)
      observer.disconnect()
      void commands.closeSourceBrowser().catch(() => undefined)
    }
    // The browser has its own navigation history and is retained when this view closes.
  }, [])

  const navigate = async (url: string) => {
    setMessage(null)
    await call(commands.navigateSourceBrowser, url)
    setAddress(url)
  }
  const submit = (event: FormEvent) => {
    event.preventDefault()
    void navigate(address).catch(() => undefined)
  }
  const translate = async () => {
    if (busy || processing) return
    setBusy(true)
    setMessage(null)
    const channel = new Channel<SourceProgress>()
    channel.onmessage = setProgress
    try {
      const result = await call(commands.prepareSourceTranslation, revalidate, replace, channel)
      await call(commands.closeSourceBrowser)
      await refresh(projectKey, pagesKey, pageKey)
      if (!result.cached) {
        await call(commands.process, { scope: 'project' }, { operation: 'full' })
      }
      useKoharuStore.getState().setSettingsOpen(false)
      setOpen(false)
    } catch (error) {
      setMessage(error instanceof Error ? error.message : t('source.failed'))
      // Restore the source surface if starting the pipeline failed after hiding it.
      if (container.current) {
        const rect = container.current.getBoundingClientRect()
        void commands
          .openSourceBrowser({ x: rect.x, y: rect.y, width: rect.width, height: rect.height })
          .catch(() => undefined)
      }
    } finally {
      setProgress(null)
      setBusy(false)
    }
  }
  const clear = async () => {
    setBusy(true)
    try {
      await call(commands.clearSourceCache)
      setMessage(t('source.cleared'))
    } catch (error) {
      setMessage(error instanceof Error ? error.message : t('source.failed'))
    } finally {
      setBusy(false)
    }
  }
  const changeLanguage = async (language: string | null) => {
    if (!language || !preferences || busy || processing) return
    setBusy(true)
    try {
      receivePreferences(
        await savePreferences(
          {
            ...preferences.pipeline,
            translation: { ...preferences.pipeline.translation, target_language: language },
          },
          preferences.providers,
          preferences.typesetting,
        ),
      )
    } catch {
      setMessage(t('source.failed'))
    } finally {
      setBusy(false)
    }
  }
  const languagePopup = (open: boolean) => {
    // DOM popovers cannot draw above a native child webview. Keep the page alive
    // but hide its surface until the language picker closes.
    if (open) void commands.closeSourceBrowser().catch(() => undefined)
    else if (container.current) {
      const rect = container.current.getBoundingClientRect()
      void commands
        .openSourceBrowser({ x: rect.x, y: rect.y, width: rect.width, height: rect.height })
        .catch(() => undefined)
    }
  }

  return (
    <main className='flex min-h-0 flex-1 flex-col bg-[var(--surface-panel)]'>
      <div className='flex flex-wrap items-center gap-2 border-b border-border/60 px-3 py-2'>
        <div className='flex items-center gap-0.5'>
          {(
            [
              ['back', ArrowLeft, 'source.back'],
              ['forward', ArrowRight, 'source.forward'],
              ['reload', RefreshCw, 'source.reload'],
            ] as const
          ).map(([action, Icon, label]) => (
            <Button
              key={action}
              variant='ghost'
              size='icon-sm'
              disabled={busy}
              aria-label={t(label)}
              title={t(label)}
              onClick={() => void call(commands.sourceBrowserAction, action).catch(() => undefined)}
            >
              <Icon className='size-4' />
            </Button>
          ))}
          <Button
            variant='ghost'
            size='icon-sm'
            disabled={busy}
            aria-label={t('source.home')}
            title={t('source.home')}
            onClick={() => void navigate(home).catch(() => undefined)}
          >
            <Home className='size-4' />
          </Button>
        </div>
        <form onSubmit={submit} className='flex min-w-48 flex-1 gap-1'>
          <Input
            aria-label={t('source.address')}
            value={address}
            disabled={busy}
            className='h-8 text-xs'
            onChange={(event) => setAddress(event.target.value)}
          />
          <Button type='submit' variant='secondary' size='sm' disabled={busy}>
            {t('source.go')}
          </Button>
        </form>
        <Button
          size='sm'
          disabled={busy || processing || !page?.images.length}
          onClick={() => void translate()}
        >
          {busy ? (
            <LoaderCircle className='size-4 animate-spin' />
          ) : (
            <Languages className='size-4' />
          )}
          {t('source.translate')}
        </Button>
        <Button
          variant='outline'
          size='sm'
          disabled={busy || processing}
          onClick={() => void clear()}
          title={t('source.cacheHelp')}
        >
          <Trash2 className='size-3.5' />
          {t('source.clear')}
        </Button>
      </div>
      <div className='flex flex-wrap items-center gap-x-5 gap-y-1 border-b border-border/40 px-4 py-2 text-xs text-muted-foreground'>
        {preferences && (
          <div className='flex items-center gap-2'>
            <span>{t('model.targetLanguage')}</span>
            <Select
              value={preferences.pipeline.translation.target_language}
              items={Object.fromEntries(languages.map((language) => [language.tag, language.name]))}
              disabled={busy || processing}
              onOpenChange={languagePopup}
              onValueChange={(language) => void changeLanguage(language)}
            >
              <SelectTrigger className='h-7 w-36 text-xs' aria-label={t('model.targetLanguage')}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {languages.map((language) => (
                  <SelectItem key={language.tag} value={language.tag}>
                    {language.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        )}
        <label className='flex cursor-pointer items-center gap-2'>
          <input
            type='checkbox'
            checked={revalidate}
            disabled={busy}
            onChange={(event) => setRevalidate(event.target.checked)}
          />
          {t('source.revalidate')}
        </label>
        <label className='flex cursor-pointer items-center gap-2' title={t('source.replaceHelp')}>
          <input
            type='checkbox'
            checked={replace}
            disabled={busy}
            onChange={(event) => setReplace(event.target.checked)}
          />
          {t('source.replace')}
        </label>
        <span
          className='ml-auto flex min-w-0 items-center gap-1.5'
          role='status'
          aria-live='polite'
        >
          <BookOpen className='size-3.5 shrink-0' />
          {progress
            ? t('source.progress', progress)
            : (message ??
              pageError ??
              (page?.images.length
                ? t('source.pages', { count: page.images.length })
                : t('source.chooseChapter')))}
        </span>
      </div>
      <div ref={container} className='min-h-0 flex-1' aria-label={t('source.browser')} />
    </main>
  )
}
