'use client'

import { useMutationState } from '@tanstack/react-query'
import {
  Activity,
  CheckCircle2,
  CircleAlert,
  Clock,
  Download,
  Languages,
  LoaderCircle,
  RotateCcw,
  ScanSearch,
  ScanText,
  Sparkles,
  Square,
  X,
} from 'lucide-react'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { formatDiskSize } from '@/lib/models'
import { send, usePages } from '@/lib/queries'
import { dismissDownloadFromDesktop, useKoharuStore, type Job } from '@/lib/store'
import {
  commands,
  type DownloadEntry,
  type PageId,
  type QueueOrderReason,
  type QueueScope,
  type QueuedRunSummary,
  type QueuedStep,
  type RunId,
  type RunPriority,
  type Stage,
} from '@koharu/ipc/protocol'
import { Badge } from '@koharu/ui/components/badge'
import { Button } from '@koharu/ui/components/button'
import { Progress, ProgressLabel, ProgressValue } from '@koharu/ui/components/progress'
import { ScrollArea } from '@koharu/ui/components/scroll-area'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'

export function ActivityCenter() {
  const { t } = useTranslation()
  const jobs = useKoharuStore((state) => state.jobs)
  const queueOrder = useKoharuStore((state) => state.queueOrder)
  const downloads = useKoharuStore((state) => state.downloads)
  const loading = useKoharuStore((state) => state.loading)
  const chapter = loading && !loading.error && loading.loaded < loading.total ? loading : null
  const activities = useMutationState({
    filters: { status: 'pending' },
    select: (mutation) => ({ id: mutation.mutationId, label: mutation.meta?.activity }),
  }).flatMap(({ id, label }) => (label ? [{ id, label }] : []))

  const jobList = Object.values(jobs)
  const runningJobs = jobList
    .filter((job) => job.state === 'running')
    .sort((a, b) => {
      const rankA = a.priority === 'rerun_now' ? 0 : 1
      const rankB = b.priority === 'rerun_now' ? 0 : 1
      return rankA !== rankB ? rankA - rankB : a.id - b.id
    })
  const failedJobs = jobList.filter((job) => job.state === 'failed')

  const queuedIds = new Set(queueOrder.map((item) => item.run))
  const fallbackQueued: QueuedRunSummary[] = jobList
    .filter(
      (job) =>
        (job.state === 'queued' || job.state === 'waiting_for_download') && !queuedIds.has(job.id),
    )
    .sort((a, b) => a.id - b.id)
    .map((job) => ({
      run: job.id,
      steps: job.total,
      scope: job.scope ?? { kind: 'project' },
      stages: job.stages,
      jobs: job.pendingSteps,
      priority: job.priority ?? 'normal',
      waiting_downloads: job.waitingDownloads,
      reason:
        job.state === 'waiting_for_download' || job.waitingDownloads.length > 0
          ? 'waiting_for_download'
          : 'fifo',
    }))
  const queuedItems = [...queueOrder, ...fallbackQueued]

  const activePendingSteps = runningJobs.flatMap((job) =>
    job.pendingSteps.map((step) => ({
      run: job.id,
      step,
      priority: job.priority,
    })),
  )
  const queuedStepCount =
    activePendingSteps.length +
    queuedItems.reduce(
      (sum, item) =>
        sum + Math.max(item.jobs.length || jobs[String(item.run)]?.pendingSteps.length || 0, 1),
      0,
    )
  const needsPages =
    runningJobs.length > 0 ||
    failedJobs.some((job) => job.failed.length > 0) ||
    activePendingSteps.length > 0 ||
    queuedItems.length > 0
  const pages = usePages(needsPages).data

  const downloadList = Object.values(downloads).sort((a, b) => {
    const rank = (entry: DownloadEntry) => {
      if (entry.state === 'Running') return 0
      if (typeof entry.state === 'object' || entry.state === 'Cancelled') return 1
      return 2
    }
    const diff = rank(a) - rank(b)
    return diff !== 0 ? diff : a.id - b.id
  })

  const hasRunningSection =
    activities.length > 0 || chapter !== null || runningJobs.length > 0 || failedJobs.length > 0
  const hasQueuedSection = activePendingSteps.length > 0 || queuedItems.length > 0
  const hasDownloadsSection = downloadList.length > 0

  if (!hasRunningSection && !hasQueuedSection && !hasDownloadsSection) {
    return (
      <div
        aria-label={t('activity.title')}
        className='flex min-h-0 flex-1 flex-col items-center justify-center gap-2 px-6 py-8 text-center'
      >
        <Activity className='size-6 text-muted-foreground/50' aria-hidden='true' />
        <p className='text-[11px] font-medium text-foreground'>{t('activity.emptyTitle')}</p>
        <p className='text-[10px] leading-4 text-muted-foreground'>
          {t('activity.emptyDescription')}
        </p>
      </div>
    )
  }

  return (
    <ScrollArea aria-label={t('activity.title')} className='min-h-0 flex-1'>
      <div className='flex flex-col divide-y divide-border/70'>
        {hasRunningSection && (
          <section aria-label={t('activity.sections.running')} className='flex flex-col'>
            <SectionHeader
              title={t('activity.sections.running')}
              count={runningJobs.length + activities.length + (chapter ? 1 : 0)}
            />
            <div className='divide-y divide-border/50'>
              {activities.map((activity) => (
                <div
                  key={activity.id}
                  role='status'
                  className='grid grid-cols-[1rem_minmax(0,1fr)] items-center gap-x-2.5 px-3 py-2.5'
                >
                  <LoaderCircle
                    className='size-3.5 animate-spin justify-self-center text-primary motion-reduce:animate-none'
                    aria-hidden='true'
                  />
                  <span className='truncate text-[11px] font-medium'>{t(activity.label)}</span>
                </div>
              ))}
              {chapter && (
                <ProgressRow
                  icon={<Download className='mt-0.5 size-3.5 justify-self-center text-primary' />}
                  label={t('activity.loadingPages', {
                    loaded: chapter.loaded,
                    total: chapter.total,
                  })}
                  completed={chapter.loaded}
                  total={chapter.total}
                />
              )}
              {runningJobs.map((job) => (
                <RunningJobItem key={job.id} job={job} pages={pages} />
              ))}
              {failedJobs.map((job) => (
                <FailedJobItem key={job.id} job={job} pages={pages} />
              ))}
            </div>
          </section>
        )}

        {hasQueuedSection && (
          <section aria-label={t('activity.sections.queued')} className='flex flex-col'>
            <SectionHeader title={t('activity.sections.queued')} count={queuedStepCount} />
            <div className='divide-y divide-border/50'>
              {activePendingSteps.map(({ run, step, priority }) => (
                <QueuedStepItem
                  key={`${run}:${step.stage}:${step.page ?? 'metadata'}`}
                  run={run}
                  step={step}
                  pages={pages}
                  priority={priority}
                />
              ))}
              {queuedItems.map((item, index) => (
                <QueuedRunGroup
                  key={item.run}
                  item={item}
                  index={index}
                  job={jobs[String(item.run)]}
                  pages={pages}
                />
              ))}
            </div>
          </section>
        )}

        {hasDownloadsSection && (
          <section aria-label={t('activity.sections.downloads')} className='flex flex-col'>
            <SectionHeader title={t('activity.sections.downloads')} count={downloadList.length} />
            <div className='divide-y divide-border/50'>
              {downloadList.map((download) => (
                <DownloadItem key={download.id} download={download} />
              ))}
            </div>
          </section>
        )}
      </div>
    </ScrollArea>
  )
}

function SectionHeader({ title, count }: { title: string; count: number }) {
  return (
    <header className='flex items-center justify-between bg-muted/30 px-3 py-1.5'>
      <h3 className='text-[10px] font-semibold tracking-[0.08em] text-muted-foreground uppercase'>
        {title}
      </h3>
      {count > 0 && (
        <span className='text-[10px] font-medium text-muted-foreground tabular-nums'>{count}</span>
      )}
    </header>
  )
}

function RunningJobItem({ job, pages }: { job: Job; pages: Array<{ id: PageId }> | undefined }) {
  const { t } = useTranslation()
  const pageDetail = formatRunningDetail(job, pages, t)

  return (
    <ProgressRow
      icon={
        <LoaderCircle
          className='mt-0.5 size-3.5 animate-spin justify-self-center text-primary motion-reduce:animate-none'
          aria-hidden='true'
        />
      }
      label={
        job.stage ? t(`phase.${job.stage}`, { defaultValue: job.stage }) : t('activity.processing')
      }
      badge={
        job.model ? (
          <Tooltip>
            <TooltipTrigger
              render={
                <Badge
                  variant='secondary'
                  className='h-4 max-w-32 min-w-0 shrink px-1.5 text-[9px] font-normal'
                />
              }
            >
              <span className='truncate'>{job.model}</span>
            </TooltipTrigger>
            <TooltipContent side='left'>{job.model}</TooltipContent>
          </Tooltip>
        ) : undefined
      }
      detail={pageDetail}
      completed={job.completed}
      total={job.total}
      action={
        <Button
          size='icon-xs'
          variant='ghost'
          className='-mt-0.5'
          aria-label={t('activity.stopRun')}
          onClick={() => send(commands.cancelRun, job.id, null, null)}
        >
          <Square className='size-2.5 fill-current' />
        </Button>
      }
    />
  )
}

function FailedJobItem({ job, pages }: { job: Job; pages: Array<{ id: PageId }> | undefined }) {
  const { t } = useTranslation()
  const dismiss = useKoharuStore((state) => state.dismissJob)
  const detail = job.failed
    .map((id) => pages?.findIndex((page) => page.id === id) ?? -1)
    .filter((index) => index >= 0)
    .map((index) => t('navigator.pageNumber', { number: index + 1 }))
    .join(', ')

  return (
    <Failure
      message={job.error || t('activity.processingFailed')}
      detail={detail}
      onDismiss={() => dismiss(String(job.id))}
    />
  )
}

function StageIcon({ stage, className }: { stage: Stage; className?: string }) {
  switch (stage) {
    case 'detect':
      return <ScanSearch className={className} aria-hidden='true' />
    case 'recognize':
      return <ScanText className={className} aria-hidden='true' />
    case 'inpaint':
      return <Sparkles className={className} aria-hidden='true' />
    case 'translate':
      return <Languages className={className} aria-hidden='true' />
  }
}

function QueuedStepItem({
  run,
  step,
  pages,
  priority,
  indented = false,
}: {
  run: RunId
  step: QueuedStep
  pages: Array<{ id: PageId }> | undefined
  priority?: RunPriority | null
  indented?: boolean
}) {
  const { t } = useTranslation()
  const label = formatStepLabel(step, pages, t)

  return (
    <div
      className={
        indented
          ? 'grid grid-cols-[1rem_minmax(0,1fr)_1.5rem] items-center gap-x-2.5 py-1.5 pr-3 pl-5'
          : 'grid grid-cols-[1rem_minmax(0,1fr)_1.5rem] items-center gap-x-2.5 px-3 py-2'
      }
    >
      <StageIcon
        stage={step.stage}
        className='size-3.5 justify-self-center text-muted-foreground'
      />
      <div className='flex min-w-0 items-center gap-1.5'>
        <span className='min-w-0 truncate text-[11px] font-medium text-foreground'>{label}</span>
        {priority === 'rerun_now' && (
          <Badge variant='default' className='h-4 shrink-0 px-1.5 text-[9px]'>
            {t('activity.rerunNow')}
          </Badge>
        )}
      </div>
      <Button
        size='icon-xs'
        variant='ghost'
        aria-label={t('activity.cancelJob', { job: label })}
        onClick={() => send(commands.cancelRun, run, step.page, step.stage)}
      >
        <X className='size-3' />
      </Button>
    </div>
  )
}

function QueuedRunGroup({
  item,
  index,
  job,
  pages,
}: {
  item: QueuedRunSummary
  index: number
  job?: Job
  pages: Array<{ id: PageId }> | undefined
}) {
  const { t } = useTranslation()
  const waiting = item.waiting_downloads.length > 0 || job?.state === 'waiting_for_download'
  const scopeLabel = formatScope(item.scope, t)
  const stagesLabel = item.stages
    .map((stage: Stage) => t(`phase.${stage}`, { defaultValue: stage }))
    .join(', ')
  const summary = stagesLabel ? `${scopeLabel} · ${stagesLabel}` : scopeLabel
  const reasonLabel = formatReason(item.reason, t)
  const steps = item.jobs.length > 0 ? item.jobs : (job?.pendingSteps ?? [])

  return (
    <div className='flex flex-col'>
      <div className='grid grid-cols-[1rem_minmax(0,1fr)_1.5rem] items-start gap-x-2.5 bg-muted/15 px-3 py-2'>
        <Clock
          className='mt-0.5 size-3.5 justify-self-center text-muted-foreground'
          aria-hidden='true'
        />
        <div className='flex min-w-0 flex-col gap-1'>
          <div className='flex flex-wrap items-center gap-1.5'>
            <Badge variant='outline' className='h-4 px-1.5 text-[9px] tabular-nums'>
              {t('activity.queueOrder', { index: index + 1 })}
            </Badge>
            {item.priority === 'rerun_now' && (
              <Badge variant='default' className='h-4 px-1.5 text-[9px]'>
                {t('activity.rerunNow')}
              </Badge>
            )}
            {waiting && (
              <Badge variant='secondary' className='h-4 px-1.5 text-[9px]'>
                {t('activity.waitingForDownload')}
              </Badge>
            )}
          </div>
          <p className='truncate text-[11px] font-medium text-foreground'>{summary}</p>
          {reasonLabel && (
            <p className='truncate text-[10px] text-muted-foreground'>{reasonLabel}</p>
          )}
        </div>
        <Button
          size='icon-xs'
          variant='ghost'
          className='-mt-0.5'
          aria-label={t('activity.cancelRun')}
          onClick={() => send(commands.cancelRun, item.run, null, null)}
        >
          <X className='size-3' />
        </Button>
      </div>
      {steps.length > 0 && (
        <div className='divide-y divide-border/40 border-t border-border/40'>
          {steps.map((step) => (
            <QueuedStepItem
              key={`${item.run}:${step.stage}:${step.page ?? 'metadata'}`}
              run={item.run}
              step={step}
              pages={pages}
              indented
            />
          ))}
        </div>
      )}
    </div>
  )
}

function DownloadItem({ download }: { download: DownloadEntry }) {
  const { t, i18n } = useTranslation()
  const stageBadge = formatStageBadge(download.stage, t)
  const modelName = download.model_name || t('activity.modelDownload')

  if (download.state === 'Running') {
    const completedText = formatDiskSize(download.completed, t, i18n.language)
    const totalText = download.total > 0 ? formatDiskSize(download.total, t, i18n.language) : null
    const progressDetail = totalText
      ? t('activity.downloadProgress', { completed: completedText, total: totalText })
      : completedText
    const filesDetail =
      download.files_total > 1
        ? t('activity.downloadFiles', {
            done: download.files_done,
            total: download.files_total,
          })
        : null
    const speedDetail =
      download.bytes_per_sec > 0
        ? t('activity.downloadSpeed', {
            speed: formatDiskSize(download.bytes_per_sec, t, i18n.language),
          })
        : null
    const detail = [progressDetail, filesDetail, speedDetail].filter(Boolean).join(' · ')

    return (
      <div className='grid grid-cols-[1rem_minmax(0,1fr)_1.5rem] items-start gap-x-2.5 px-3 py-2.5'>
        <Download className='mt-0.5 size-3.5 justify-self-center text-primary' aria-hidden='true' />
        <Progress
          value={download.total > 0 ? Math.min(download.completed, download.total) : null}
          max={download.total || 100}
          locale={i18n.language}
          className='min-w-0 gap-x-2 gap-y-1.5'
        >
          <div className='flex min-w-0 flex-1 items-center gap-1.5'>
            <Tooltip>
              <TooltipTrigger
                render={
                  <ProgressLabel className='min-w-0 flex-1 truncate text-[11px] font-medium' />
                }
              >
                {modelName}
              </TooltipTrigger>
              <TooltipContent side='left'>{modelName}</TooltipContent>
            </Tooltip>
            {stageBadge && (
              <Badge variant='secondary' className='h-4 shrink-0 px-1.5 text-[9px]'>
                {stageBadge}
              </Badge>
            )}
          </div>
          <ProgressValue className='shrink-0 text-[10px] text-foreground' />
          {detail && <p className='w-full truncate text-[10px] text-muted-foreground'>{detail}</p>}
        </Progress>
        <Button
          size='icon-xs'
          variant='ghost'
          className='-mt-0.5'
          aria-label={t('activity.cancelDownload')}
          onClick={() => send(commands.cancelDownload, download.id)}
        >
          <X className='size-3' />
        </Button>
      </div>
    )
  }

  if (download.state === 'Finished') {
    const sizeText = download.total > 0 ? formatDiskSize(download.total, t, i18n.language) : null
    const detail = sizeText
      ? `${t('activity.downloadFinished')} · ${sizeText}`
      : t('activity.downloadFinished')

    return (
      <div className='grid grid-cols-[1rem_minmax(0,1fr)_1.5rem] items-start gap-x-2.5 px-3 py-2.5'>
        <CheckCircle2
          className='mt-0.5 size-3.5 justify-self-center text-emerald-500'
          aria-hidden='true'
        />
        <div className='flex min-w-0 flex-col gap-0.5'>
          <div className='flex min-w-0 items-center gap-1.5'>
            <Tooltip>
              <TooltipTrigger
                render={<span className='min-w-0 flex-1 truncate text-[11px] font-medium' />}
              >
                {modelName}
              </TooltipTrigger>
              <TooltipContent side='left'>{modelName}</TooltipContent>
            </Tooltip>
            {stageBadge && (
              <Badge variant='secondary' className='h-4 shrink-0 px-1.5 text-[9px]'>
                {stageBadge}
              </Badge>
            )}
          </div>
          <p className='truncate text-[10px] text-muted-foreground'>{detail}</p>
        </div>
        <Button
          size='icon-xs'
          variant='ghost'
          className='-mt-0.5'
          aria-label={t('activity.dismissDownload')}
          onClick={() => dismissDownloadFromDesktop(download.id)}
        >
          <X className='size-3' />
        </Button>
      </div>
    )
  }

  const errorText =
    typeof download.state === 'object'
      ? download.state.Failed || t('activity.downloadFailed')
      : t('activity.downloadCancelled')

  return (
    <div className='grid grid-cols-[1rem_minmax(0,1fr)_auto] items-start gap-x-2.5 px-3 py-2.5'>
      <CircleAlert
        className='mt-0.5 size-3.5 justify-self-center text-destructive'
        aria-hidden='true'
      />
      <div className='flex min-w-0 flex-col gap-0.5'>
        <div className='flex min-w-0 items-center gap-1.5'>
          <Tooltip>
            <TooltipTrigger
              render={<span className='min-w-0 flex-1 truncate text-[11px] font-medium' />}
            >
              {modelName}
            </TooltipTrigger>
            <TooltipContent side='left'>{modelName}</TooltipContent>
          </Tooltip>
          {stageBadge && (
            <Badge variant='secondary' className='h-4 shrink-0 px-1.5 text-[9px]'>
              {stageBadge}
            </Badge>
          )}
        </div>
        <p className='text-[10px] [overflow-wrap:anywhere] text-destructive'>{errorText}</p>
      </div>
      <div className='-mt-0.5 flex items-center gap-0.5'>
        <Button
          size='icon-xs'
          variant='ghost'
          aria-label={t('activity.retryDownload')}
          onClick={() => send(commands.retryDownload, download.id)}
        >
          <RotateCcw className='size-3' />
        </Button>
        <Button
          size='icon-xs'
          variant='ghost'
          aria-label={t('activity.dismissDownload')}
          onClick={() => dismissDownloadFromDesktop(download.id)}
        >
          <X className='size-3' />
        </Button>
      </div>
    </div>
  )
}

/** A row of work under way: its name and how far along it is, in the interface's language, over
 * its bar, which runs on its own while the total is unknown. */
function ProgressRow({
  icon,
  label,
  badge,
  detail,
  completed,
  total,
  action,
}: {
  icon: ReactNode
  label: string
  badge?: ReactNode
  detail?: string
  completed: number | null
  total: number
  action?: ReactNode
}) {
  const { i18n } = useTranslation()
  return (
    <div className='grid grid-cols-[1rem_minmax(0,1fr)_1.5rem] items-start gap-x-2.5 px-3 py-2.5'>
      {icon}
      <Progress
        value={completed !== null && total > 0 ? Math.min(completed, total) : null}
        max={total || 100}
        locale={i18n.language}
        className='min-w-0 gap-x-2 gap-y-1.5'
      >
        <div className='flex min-w-0 flex-1 items-center gap-1.5'>
          <ProgressLabel className='shrink-0 text-[11px] font-medium'>{label}</ProgressLabel>
          {badge}
        </div>
        <ProgressValue className='shrink-0 text-[10px] text-foreground' />
        {detail && <p className='w-full truncate text-[10px] text-muted-foreground'>{detail}</p>}
      </Progress>
      {action ?? <span aria-hidden='true' />}
    </div>
  )
}

/** A failure's row: why, and under it where, as a running row names its page. */
function Failure({
  message,
  detail,
  onDismiss,
}: {
  message: string
  detail?: string
  onDismiss: () => void
}) {
  const { t } = useTranslation()
  return (
    <div className='grid grid-cols-[1rem_minmax(0,1fr)_1.5rem] items-start gap-x-2.5 px-3 py-2.5 text-[11px]'>
      <CircleAlert className='mt-0.5 size-3.5 justify-self-center text-destructive' />
      <div className='min-w-0'>
        <p className='[overflow-wrap:anywhere] text-destructive'>{message}</p>
        {detail ? (
          <p className='mt-0.5 truncate text-[10px] text-muted-foreground'>{detail}</p>
        ) : null}
      </div>
      <Button
        size='icon-xs'
        variant='ghost'
        className='-mt-0.5'
        aria-label={t('activity.dismiss')}
        onClick={onDismiss}
      >
        <X className='size-3' />
      </Button>
    </div>
  )
}

function formatStepTarget(
  page: PageId | null,
  pages: Array<{ id: PageId }> | undefined,
  t: ReturnType<typeof useTranslation>['t'],
): string {
  if (page === null) return t('activity.queueScope.metadata')
  const index = pages?.findIndex((candidate) => candidate.id === page) ?? -1
  return index >= 0 ? t('navigator.pageNumber', { number: index + 1 }) : page
}

function formatStepLabel(
  step: QueuedStep,
  pages: Array<{ id: PageId }> | undefined,
  t: ReturnType<typeof useTranslation>['t'],
): string {
  return t('activity.queuedStep', {
    target: formatStepTarget(step.page, pages, t),
    stage: t(`phase.${step.stage}`, { defaultValue: step.stage }),
  })
}

function formatRunningDetail(
  job: Job,
  pages: Array<{ id: PageId }> | undefined,
  t: ReturnType<typeof useTranslation>['t'],
): string | undefined {
  if (job.runningSteps.length > 1) {
    const numbers = job.runningSteps
      .map((step) => (step.page ? (pages?.findIndex((p) => p.id === step.page) ?? -1) + 1 : 0))
      .filter((n) => n > 0)
      .sort((a, b) => a - b)
    if (numbers.length > 1) {
      const contiguous = numbers.every((num, idx) => idx === 0 || num === numbers[idx - 1] + 1)
      if (contiguous) {
        return t('activity.pageRange', { from: numbers[0], to: numbers[numbers.length - 1] })
      }
      return numbers.map((number) => t('navigator.pageNumber', { number })).join(', ')
    }
  }
  const activePage = job.runningSteps[0]?.page ?? job.page
  const activeStage = job.runningSteps[0]?.stage ?? job.stage
  if (activePage === null) {
    return activeStage === 'translate' ? t('activity.queueScope.metadata') : undefined
  }
  const index = pages?.findIndex((page) => page.id === activePage) ?? -1
  return index >= 0 ? t('navigator.pageNumber', { number: index + 1 }) : undefined
}

function formatScope(scope: QueueScope, t: ReturnType<typeof useTranslation>['t']): string {
  switch (scope.kind) {
    case 'project':
      return t('activity.queueScope.project')
    case 'pages':
      return t('activity.queueScope.pages', { count: scope.count })
    case 'blocks':
      return t('activity.queueScope.blocks', { count: scope.count })
    case 'area':
      return t('activity.queueScope.area')
    case 'metadata':
      return t('activity.queueScope.metadata')
  }
}

function formatReason(
  reason: QueueOrderReason | null,
  t: ReturnType<typeof useTranslation>['t'],
): string | null {
  switch (reason) {
    case 'fifo':
      return t('activity.queueReason.fifo')
    case 'grouped_by_loaded_model':
      return t('activity.queueReason.grouped_by_loaded_model')
    case 'lower_vram_cost':
      return t('activity.queueReason.lower_vram_cost')
    case 'aged_priority':
      return t('activity.queueReason.aged_priority')
    case 'waiting_for_download':
      return t('activity.queueReason.waiting_for_download')
    case null:
      return null
  }
}

function formatStageBadge(
  stage: string | null,
  t: ReturnType<typeof useTranslation>['t'],
): string | null {
  switch (stage) {
    case 'detect':
    case 'recognize':
    case 'inpaint':
    case 'translate':
      return t(`phase.${stage}`)
    case null:
      return null
    default:
      return t('activity.stageRuntime')
  }
}
