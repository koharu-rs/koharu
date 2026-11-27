'use client'

import { useMutationState } from '@tanstack/react-query'
import { Activity, CircleAlert, LoaderCircle } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { usePage } from '@/lib/queries'
import { isActiveJob, useKoharuStore } from '@/lib/store'
import { Slider } from '@koharu/ui/components/slider'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'
import { cn } from '@koharu/ui/lib/utils'

interface StatusBarProps {
  onZoomChange: (zoom: number) => void
}

export function StatusBar({ onZoomChange }: StatusBarProps) {
  const { t } = useTranslation()
  const page = usePage().data
  // Selects the displayed percentage so translation-only camera changes do not re-render the bar.
  const zoom = useKoharuStore((state) => Math.round(state.camera.zoom * 100))
  const panel = useKoharuStore((state) => state.panel)
  const showPanel = useKoharuStore((state) => state.showPanel)
  const jobs = useKoharuStore((state) => state.jobs)
  const downloads = useKoharuStore((state) => state.downloads)
  const loading = useKoharuStore((state) => state.loading)
  const activities = useMutationState({
    filters: { status: 'pending' },
    select: (mutation) => mutation.meta?.activity,
  }).flatMap((label) => (label ? [label] : []))

  const jobList = Object.values(jobs)
  const downloadList = Object.values(downloads)
  const runningJobs = jobList.filter((job) => job.state === 'running')
  const queuedJobs = jobList.filter(
    (job) => job.state === 'queued' || job.state === 'waiting_for_download',
  )
  const runningDownloads = downloadList.filter((download) => download.state === 'Running')
  const chapter = loading && !loading.error && loading.loaded < loading.total ? loading : null

  const activeCount =
    jobList.filter(isActiveJob).length +
    runningDownloads.length +
    activities.length +
    (chapter ? 1 : 0)

  const failedCount =
    jobList.filter((job) => job.state === 'failed').length +
    downloadList.filter((download) => typeof download.state === 'object').length

  let summary = t('status.activity')
  if (activeCount > 0) {
    if (runningJobs.length > 0) {
      const lead = runningJobs[0]
      const stageLabel = lead.stage
        ? t(`phase.${lead.stage}`, { defaultValue: lead.stage })
        : t('activity.processing')
      const base = lead.model ? `${stageLabel} · ${lead.model}` : stageLabel
      summary =
        queuedJobs.length > 0
          ? t('status.activityQueued', { active: base, queued: queuedJobs.length })
          : base
    } else if (runningDownloads.length > 0) {
      const lead = runningDownloads[0]
      const base = lead.model_name || t('activity.modelDownload')
      summary =
        queuedJobs.length > 0
          ? t('status.activityQueued', { active: base, queued: queuedJobs.length })
          : base
    } else if (activities.length > 0) {
      summary = t(activities[0])
    } else if (chapter) {
      summary = t('activity.loadingPages', { loaded: chapter.loaded, total: chapter.total })
    } else {
      summary = t('status.activityActive', { count: activeCount })
    }
  } else if (failedCount > 0) {
    summary = t('status.activityFailed', { count: failedCount })
  }

  return (
    <footer className='flex h-7 shrink-0 items-center gap-3 border-t border-border/80 bg-[var(--surface-toolbar)] px-2.5 text-[10px] text-muted-foreground'>
      <div className='flex w-36 shrink-0 items-center gap-2'>
        <Slider
          aria-label={t('status.zoom')}
          min={10}
          max={800}
          step={5}
          value={Math.min(800, Math.max(10, zoom))}
          className='w-24 shrink-0 [&_[data-slot=slider-thumb]]:size-2'
          onValueChange={(value) => onZoomChange(value / 100)}
        />
        <span className='w-9 shrink-0 text-right text-foreground tabular-nums'>{zoom}%</span>
      </div>
      <div className='flex-1' />
      <Tooltip>
        <TooltipTrigger
          render={
            <button
              type='button'
              onClick={() => showPanel('activity')}
              aria-pressed={panel === 'activity'}
              aria-label={summary}
              className={cn(
                'flex h-5 max-w-56 min-w-0 items-center gap-1.5 rounded-full px-2 text-[10px] transition-colors hover:bg-foreground/[0.06] hover:text-foreground',
                panel === 'activity' && 'bg-primary/10 text-foreground',
                activeCount === 0 && failedCount > 0 && 'text-destructive',
              )}
            />
          }
        >
          {activeCount > 0 ? (
            <LoaderCircle
              className='size-3 shrink-0 animate-spin text-primary motion-reduce:animate-none'
              aria-hidden='true'
            />
          ) : failedCount > 0 ? (
            <CircleAlert className='size-3 shrink-0 text-destructive' aria-hidden='true' />
          ) : (
            <Activity className='size-3 shrink-0' aria-hidden='true' />
          )}
          <span className='min-w-0 truncate'>{summary}</span>
        </TooltipTrigger>
        <TooltipContent side='top' className='flex-col items-start gap-0.5'>
          <p>{summary}</p>
          <p className='opacity-70'>{t('status.openActivity')}</p>
        </TooltipContent>
      </Tooltip>
      {page && (
        <span className='shrink-0 tabular-nums'>
          {page.size.width} × {page.size.height} px
        </span>
      )}
    </footer>
  )
}
