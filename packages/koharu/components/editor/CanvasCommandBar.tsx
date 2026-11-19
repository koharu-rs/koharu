'use client'

import { ChevronDown, Eye, EyeOff, Minus } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { InferenceControl } from '@/components/editor/InferenceControl'
import { call } from '@/lib/backend'
import { usePage, useProject } from '@/lib/queries'
import { pipelineStages, useKoharuStore, type PipelineScope } from '@/lib/store'
import { commands, type PageSummary, type Stage, type Target } from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from '@koharu/ui/components/dropdown-menu'

function SfxMenu({
  pages,
  project,
  disabled,
}: {
  pages: PageSummary[]
  project?: boolean
  disabled: boolean
}) {
  const { t } = useTranslation()
  const total = pages.reduce((count, page) => count + page.sfx_total, 0)
  const visible = pages.reduce((count, page) => count + page.sfx_visible, 0)
  const state = visible === 0 ? 'hide' : visible === total ? 'show' : 'mixed'
  const Icon = state === 'mixed' ? Minus : visible ? Eye : EyeOff
  const scope = t(project ? 'sfxVisibility.project' : 'sfxVisibility.page')
  const show = t(project ? 'sfxVisibility.showProject' : 'sfxVisibility.showPage')
  const hide = t(project ? 'sfxVisibility.hideProject' : 'sfxVisibility.hidePage')
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button variant='ghost' size='sm' className='min-w-0 shrink gap-1 px-1.5 text-[11px]' />
        }
        disabled={disabled || !total}
      >
        <Icon className='size-3.5' />
        <span className='truncate'>
          {scope}
          {state === 'mixed' && ` · ${t('sfxVisibility.mixed')}`}
        </span>
        <ChevronDown className='size-3 text-muted-foreground' />
      </DropdownMenuTrigger>
      <DropdownMenuContent className='w-max'>
        <DropdownMenuRadioGroup
          value={state}
          onValueChange={(value) => {
            if (value !== state)
              void call(commands.edit, {
                label: value === 'show' ? show : hide,
                ops: [
                  {
                    op: 'set_sfx_visibility',
                    page: project ? null : pages[0]!.id,
                    visible: value === 'show',
                  },
                ],
              }).catch(() => undefined)
          }}
        >
          <DropdownMenuRadioItem value='show' closeOnClick>
            <Eye />
            {show}
          </DropdownMenuRadioItem>
          <DropdownMenuRadioItem value='hide' closeOnClick>
            <EyeOff />
            {hide}
          </DropdownMenuRadioItem>
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

export function CanvasCommandBar() {
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

  return (
    <header className='flex h-10 shrink-0 items-center gap-2 border-b border-border/80 bg-[var(--surface-toolbar)] px-2.5'>
      <div className='flex min-w-0 flex-1 items-center gap-1 overflow-hidden'>
        <SfxMenu pages={pages.filter((item) => item.id === page?.id)} disabled={Boolean(running)} />
        <SfxMenu pages={pages} project disabled={Boolean(running)} />
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
