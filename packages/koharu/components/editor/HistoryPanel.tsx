'use client'

import {
  ALargeSmall,
  ArrowUpDown,
  Brush,
  Camera,
  Cpu,
  Droplet,
  Eraser,
  Eye,
  FileInput,
  FileText,
  FolderOpen,
  History as HistoryIcon,
  Languages,
  Move,
  Pencil,
  Trash,
  Trash2,
  Type,
  type LucideIcon,
} from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { call } from '@/lib/backend'
import { pageKey, pagesKey, projectKey, refresh, useProject } from '@/lib/queries'
import { commands, type HistoryEntryInfo, type HistoryName } from '@koharu/bridge/protocol'
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
import { Button } from '@koharu/ui/components/button'
import { ScrollArea } from '@koharu/ui/components/scroll-area'

const STATE_ICON: Record<HistoryName, LucideIcon> = {
  open: FolderOpen,
  import_pages: FileInput,
  rename_page: Pencil,
  delete_pages: Trash,
  move_page: ArrowUpDown,
  add_text: Type,
  source_text: FileText,
  translation: Languages,
  typography: ALargeSmall,
  geometry: Move,
  transform: Move,
  toggle_layer: Eye,
  opacity: Droplet,
  delete_layers: Trash2,
  move_layer: Move,
  brush: Brush,
  erase: Eraser,
  pipeline_stage: Cpu,
  snapshot_restore: Camera,
}

export function HistoryPanel() {
  const { t } = useTranslation()
  const { data: project } = useProject()
  const history = project?.history
  const list = useRef<HTMLUListElement>(null)
  const [clearOpen, setClearOpen] = useState(false)

  useEffect(() => {
    list.current
      ?.querySelector('[data-current="true"]')
      ?.scrollIntoView({ block: 'nearest' })
  }, [history?.cursor, history?.entries.length])

  if (!history) return null

  const refreshProject = () => refresh(projectKey, pagesKey, pageKey).catch(() => undefined)
  const jump = (index: number) => {
    if (index === history.cursor) return
    void call(commands.historyGoTo, index).then(refreshProject).catch(() => undefined)
  }

  const label = (entry: HistoryEntryInfo) => {
    const base = t(`history.state.${entry.name}`)
    if (!entry.detail) return base
    const detail =
      entry.name === 'pipeline_stage' ? t(`phase.${entry.detail}`, entry.detail) : entry.detail
    return `${base}: ${detail}`
  }

  return (
    <div className='flex h-full min-h-0 flex-col'>
      {history.snapshots.length > 0 && (
        <div className='shrink-0 border-b border-border/80 px-2 py-1.5'>
          <div className='px-1 pb-1 text-[9px] font-medium tracking-wide text-muted-foreground uppercase'>
            {t('history.snapshots')}
          </div>
          <ul className='grid gap-px' aria-label={t('history.snapshots')}>
            {history.snapshots.map((snapshot) => (
              <li key={snapshot.id} className='group flex items-center'>
                <button
                  type='button'
                  className='flex min-w-0 flex-1 items-center gap-2 rounded px-1.5 py-1 text-left text-[11px] hover:bg-accent/60'
                  onClick={() =>
                    void call(commands.snapshotRestore, snapshot.id)
                      .then(refreshProject)
                      .catch(() => undefined)
                  }
                  title={t('history.restore')}
                >
                  <Camera className='size-3 shrink-0 text-muted-foreground' />
                  <span className='truncate'>{snapshot.name}</span>
                </button>
                <Button
                  variant='ghost'
                  size='sm'
                  className='invisible size-6 shrink-0 group-hover:visible'
                  aria-label={t('history.deleteSnapshot')}
                  onClick={() =>
                    void call(commands.snapshotDelete, snapshot.id)
                      .then(refreshProject)
                      .catch(() => undefined)
                  }
                >
                  <Trash2 className='size-3' />
                </Button>
              </li>
            ))}
          </ul>
        </div>
      )}

      <ScrollArea className='min-h-0 flex-1'>
        <ul ref={list} className='grid gap-px px-2 py-1.5' aria-label={t('history.title')}>
          {history.entries.map((entry) => {
            const Icon = STATE_ICON[entry.name] ?? HistoryIcon
            return (
              <li key={entry.index}>
                <button
                  type='button'
                  data-current={entry.current}
                  className={
                    'flex w-full items-center gap-2 rounded px-1.5 py-1 text-left text-[11px] hover:bg-accent/60 ' +
                    (entry.current
                      ? 'bg-accent/80 font-medium'
                      : entry.undone
                        ? 'text-muted-foreground italic'
                        : '')
                  }
                  onClick={() => jump(entry.index)}
                >
                  <Icon className='size-3 shrink-0 text-muted-foreground' />
                  <span className='truncate'>{label(entry)}</span>
                </button>
              </li>
            )
          })}
        </ul>
      </ScrollArea>

      <div className='flex shrink-0 items-center justify-end gap-1 border-t border-border/80 px-2 py-1.5'>
        <Button
          variant='ghost'
          size='sm'
          className='size-7'
          title={t('history.snapshot')}
          aria-label={t('history.snapshot')}
          onClick={() =>
            void call(commands.snapshotCreate, null).then(refreshProject).catch(() => undefined)
          }
        >
          <Camera className='size-3.5' />
        </Button>
        <AlertDialog open={clearOpen} onOpenChange={setClearOpen}>
          <Button
            variant='ghost'
            size='sm'
            className='size-7'
            title={t('history.clear')}
            aria-label={t('history.clear')}
            disabled={history.entries.length <= 1}
            onClick={() => setClearOpen(true)}
          >
            <Trash2 className='size-3.5' />
          </Button>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogMedia className='bg-destructive/10 text-destructive'>
                <Trash2 className='size-5' />
              </AlertDialogMedia>
              <AlertDialogTitle>{t('history.clearConfirmTitle')}</AlertDialogTitle>
              <AlertDialogDescription>
                {t('history.clearConfirmDescription')}
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>{t('common.cancel')}</AlertDialogCancel>
              <AlertDialogAction
                variant='destructive'
                onClick={() =>
                  void call(commands.historyClear).then(refreshProject).catch(() => undefined)
                }
              >
                {t('history.clearConfirmAction')}
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      </div>
    </div>
  )
}
