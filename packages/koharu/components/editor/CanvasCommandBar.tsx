'use client'

import { Eye, EyeOff } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { InferenceControl } from '@/components/editor/InferenceControl'
import { call } from '@/lib/backend'
import { usePage, useProject } from '@/lib/queries'
import { pipelineStages, useKoharuStore, type PipelineScope } from '@/lib/store'
import { commands, type Stage, type Target } from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'

export function CanvasCommandBar() {
  const { t } = useTranslation()
  const page = usePage().data
  const project = useProject().data
  const pages = project?.pages ?? []
  const selectedPages = useKoharuStore((state) => state.selectedPages)
  const jobs = useKoharuStore((state) => state.jobs)
  const running = Object.values(jobs).find((job) => job.state === 'running')

  const lastSelected = pages.findLastIndex((item) => selectedPages.includes(item.id))
  const anchor = lastSelected >= 0 ? lastSelected : pages.findIndex((item) => item.id === page?.id)
  const eligible = pages.filter((item) => !item.excluded)
  const selected = eligible.filter((item) => selectedPages.includes(item.id)).map((item) => item.id)
  const after =
    anchor < 0
      ? []
      : pages
          .slice(anchor + 1)
          .filter((item) => !item.excluded)
          .map((item) => item.id)

  const run = (selection: PipelineScope, stages: Stage[], resume: boolean) => {
    const target: Target =
      selection === 'project'
        ? 'project'
        : {
            pages: selection === 'after-selected-pages' ? after : selected,
          }
    // A run takes its stages in the pipeline's order, whichever order they were picked in.
    const ordered = pipelineStages.filter((stage) => stages.includes(stage))
    void call(commands.runPipeline, target, ordered, resume).catch(() => undefined)
  }

  const hidePage = Boolean(page?.hide_sfx)
  const hideProject = Boolean(project?.hideSfx)
  const pageLabel = t(hidePage ? 'sfxVisibility.showPage' : 'sfxVisibility.hidePage')
  const projectLabel = t(hideProject ? 'sfxVisibility.showProject' : 'sfxVisibility.hideProject')
  const PageIcon = hidePage ? EyeOff : Eye
  const ProjectIcon = hideProject ? EyeOff : Eye

  return (
    <header className='flex h-10 shrink-0 items-center gap-2 border-b border-border/80 bg-[var(--surface-toolbar)] px-2.5'>
      <div className='flex min-w-0 flex-1 items-center gap-1 overflow-hidden'>
        <Button
          type='button'
          variant='ghost'
          size='sm'
          className='min-w-0 shrink gap-1 px-1.5 text-[11px]'
          title={pageLabel}
          aria-label={pageLabel}
          disabled={!page || hideProject || Boolean(running)}
          onClick={() => {
            if (page)
              void call(commands.edit, {
                label: pageLabel,
                ops: [{ op: 'set_page_options', page: page.id, hide_sfx: !hidePage }],
              }).catch(() => undefined)
          }}
        >
          <PageIcon className='size-3.5' />
          <span className='truncate'>{t('sfxVisibility.page')}</span>
        </Button>
        <Button
          type='button'
          variant='ghost'
          size='sm'
          className='min-w-0 shrink gap-1 px-1.5 text-[11px]'
          title={projectLabel}
          aria-label={projectLabel}
          disabled={!project || Boolean(running)}
          onClick={() =>
            void call(commands.edit, {
              label: projectLabel,
              ops: [{ op: 'set_project_sfx', hide: !hideProject }],
            }).catch(() => undefined)
          }
        >
          <ProjectIcon className='size-3.5' />
          <span className='truncate'>{t('sfxVisibility.project')}</span>
        </Button>
      </div>
      <InferenceControl
        disabled={!page || Boolean(running)}
        onRun={run}
        selectedCount={selected.length}
        afterCount={after.length}
        projectCount={eligible.length}
      />
    </header>
  )
}
