'use client'

import { getVersion } from '@tauri-apps/api/app'
import { relaunch } from '@tauri-apps/plugin-process'
import { check, type Update } from '@tauri-apps/plugin-updater'
import { CircleCheck, Download, RefreshCw } from 'lucide-react'
import { useEffect, useRef } from 'react'
import { Trans, useTranslation } from 'react-i18next'
import { create } from 'zustand'

import { Markdown } from '@/components/controls/Markdown'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogMedia,
  AlertDialogTitle,
} from '@koharu/ui/components/alert-dialog'
import { Progress, ProgressValue } from '@koharu/ui/components/progress'
import { ScrollArea } from '@koharu/ui/components/scroll-area'

type UpdateState =
  | { kind: 'latest'; version: string }
  | { kind: 'available'; update: Update }
  | { kind: 'downloading'; update: Update; downloaded: number; total: number | null }
  | { kind: 'error'; update: Update; message: string }

/** What the dialog shows, closed while it's null. */
const useUpdater = create<{ state: UpdateState | null }>()(() => ({ state: null }))

const setState = (state: UpdateState | null) => useUpdater.setState({ state })

/** Offers a newer release when there's one; a check the user `asked` for also says when there's
 * none, as it does when the check fails, such as while a release's files are still being built.
 * The check at launch and Help's can both find one; the dialog keeps the one it has. */
export async function checkForUpdates(asked: boolean): Promise<void> {
  const update = await check().catch(() => null)
  if (useUpdater.getState().state) void update?.close()
  else if (update) setState({ kind: 'available', update })
  else if (asked) setState({ kind: 'latest', version: await getVersion() })
}

export function Updater() {
  const { t, i18n } = useTranslation()
  const state = useUpdater((updater) => updater.state)
  const popup = useRef<HTMLDivElement>(null)

  useEffect(() => {
    void checkForUpdates(false)
  }, [])

  const install = (update: Update) => {
    let downloaded = 0
    let total: number | null = null
    setState({ kind: 'downloading', update, downloaded, total })
    void update
      .downloadAndInstall((event) => {
        if (event.event === 'Started') {
          downloaded = 0
          total = event.data.contentLength ?? null
        } else if (event.event === 'Progress') {
          downloaded += event.data.chunkLength
        }
        setState({ kind: 'downloading', update, downloaded, total })
      })
      .then(() => relaunch())
      .catch((error: unknown) => {
        setState({
          kind: 'error',
          update,
          message: error instanceof Error ? error.message : String(error),
        })
      })
  }

  if (!state) return null

  const downloading = state.kind === 'downloading'
  const percent =
    downloading && state.total ? Math.min(100, (state.downloaded / state.total) * 100) : null

  return (
    <AlertDialog
      open
      onOpenChange={(open) => {
        if (!open && !downloading) {
          if (state.kind !== 'latest') void state.update.close()
          setState(null)
        }
      }}
    >
      {/* Focus starts on the dialog, not on the first link in the release notes. */}
      <AlertDialogContent
        ref={popup}
        initialFocus={state.kind === 'available' ? popup : true}
        className='flex max-h-[calc(100dvh-2rem)] flex-col overflow-hidden'
      >
        <AlertDialogHeader className='flex shrink-0 flex-row items-start gap-4 text-left'>
          <AlertDialogMedia className='mb-0 shrink-0'>
            {state.kind === 'latest' ? (
              <CircleCheck className='size-5' />
            ) : state.kind === 'error' ? (
              <RefreshCw className='size-5' />
            ) : (
              <Download className='size-5' />
            )}
          </AlertDialogMedia>
          <div className='min-w-0 space-y-0.5'>
            <AlertDialogTitle className='text-sm'>
              {t(`updater.${state.kind}.title`)}
            </AlertDialogTitle>
            <AlertDialogDescription className='text-xs'>
              {state.kind === 'latest' || state.kind === 'available' ? (
                <Trans
                  i18nKey={`updater.${state.kind}.description`}
                  values={{
                    version: state.kind === 'latest' ? state.version : state.update.version,
                  }}
                  components={{ strong: <strong className='font-medium text-foreground' /> }}
                />
              ) : state.kind === 'downloading' ? (
                t('updater.downloading.subtitle', { version: state.update.version })
              ) : (
                t('updater.error.description')
              )}
            </AlertDialogDescription>
          </div>
        </AlertDialogHeader>

        {state.kind === 'available' && (
          <ScrollArea
            className='h-64 max-h-[40dvh] min-h-0 rounded-lg bg-muted/45'
            viewportClassName='p-3'
          >
            {state.update.body ? (
              <Markdown className='text-muted-foreground'>{state.update.body}</Markdown>
            ) : (
              <p className='text-xs leading-5 text-muted-foreground'>{t('updater.noNotes')}</p>
            )}
          </ScrollArea>
        )}
        {state.kind === 'downloading' && (
          <Progress
            value={percent}
            locale={i18n.language}
            aria-label={t('updater.downloading.title')}
          >
            <ProgressValue className='text-xs'>{(formatted) => formatted || '…'}</ProgressValue>
          </Progress>
        )}
        {state.kind === 'error' && (
          <ScrollArea
            className='max-h-28'
            viewportClassName='text-xs leading-5 text-muted-foreground'
          >
            <p>{state.message}</p>
          </ScrollArea>
        )}

        {!downloading && (
          <AlertDialogFooter className='m-0 shrink-0 border-0 bg-transparent p-0'>
            <AlertDialogCancel>
              {state.kind === 'available' ? t('updater.later') : t('updater.close')}
            </AlertDialogCancel>
            {state.kind !== 'latest' && (
              <AlertDialogAction onClick={() => install(state.update)}>
                {state.kind === 'available' ? t('updater.update') : t('updater.retry')}
              </AlertDialogAction>
            )}
          </AlertDialogFooter>
        )}
      </AlertDialogContent>
    </AlertDialog>
  )
}
