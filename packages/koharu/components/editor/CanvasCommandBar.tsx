'use client'

import { InferenceControl } from '@/components/editor/InferenceControl'
import { send, usePage } from '@/lib/queries'
import { pipelineStages, useKoharuStore, type PipelineScope } from '@/lib/store'
import { commands, type RunPriority, type Stage, type Target } from '@koharu/ipc/protocol'

export function CanvasCommandBar() {
  const page = usePage().data
  const selectedPages = useKoharuStore((state) => state.selectedPages)

  const run = (selection: PipelineScope, stages: Stage[], priority: RunPriority = 'normal') => {
    const target: Target = selection === 'project' ? 'project' : { pages: selectedPages }
    // A run takes its stages in the pipeline's order, whichever order they were picked in.
    const ordered = pipelineStages.filter((stage) => stages.includes(stage))
    send(commands.runPipeline, target, ordered, priority)
  }

  return (
    <header className='flex h-10 shrink-0 items-center gap-2 border-b border-border/80 bg-[var(--surface-toolbar)] px-2.5'>
      <div className='min-w-0 flex-1' />
      <InferenceControl disabled={!page} onRun={run} />
    </header>
  )
}
