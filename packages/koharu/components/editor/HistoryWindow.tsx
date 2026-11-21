'use client'

import { DndContext, PointerSensor, useDraggable, useSensor, useSensors } from '@dnd-kit/core'
import { History, X } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { send, useProject } from '@/lib/queries'
import { useKoharuStore } from '@/lib/store'
import { commands, type History as Steps } from '@koharu/ipc/protocol'
import { ScrollArea } from '@koharu/ui/components/scroll-area'

/** The project's undo history in a small window over the canvas, as Photoshop's History panel
 * floats over the document: the project as it opened, then each step, oldest first, the current
 * one highlighted and the ones redo would make again dimmed; a click goes to that step. It's moved
 * by its title, from the canvas's top right, and closed by its button or View > History; it opens
 * at its place again. */
export function HistoryWindow() {
  const open = useKoharuStore((state) => state.historyOpen)
  const history = useProject().data?.history
  const [offset, setOffset] = useState({ x: 0, y: 0 })
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 4 } }))

  useEffect(() => {
    if (!open) setOffset({ x: 0, y: 0 })
  }, [open])

  if (!open || !history) return null
  return (
    <DndContext
      sensors={sensors}
      onDragEnd={({ delta }) =>
        setOffset((current) => ({ x: current.x + delta.x, y: current.y + delta.y }))
      }
    >
      <Window history={history} offset={offset} />
    </DndContext>
  )
}

function Window({ history, offset }: { history: Steps; offset: { x: number; y: number } }) {
  const { t } = useTranslation()
  const setOpen = useKoharuStore((state) => state.setHistoryOpen)
  const { attributes, listeners, setNodeRef, transform } = useDraggable({ id: 'history' })
  const current = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    current.current?.scrollIntoView({ block: 'nearest' })
  }, [history.current])

  const x = offset.x + (transform?.x ?? 0)
  const y = offset.y + (transform?.y ?? 0)
  // The project as it opened is step 0, before every step it holds.
  const states = ['', ...history.steps]
  return (
    <section
      ref={setNodeRef}
      aria-label={t('history.title')}
      data-testid='history-window'
      className='absolute top-3 right-3 z-20 flex w-56 flex-col overflow-hidden rounded-xl border border-border bg-[var(--surface-floating)] shadow-[var(--shadow-toolrail)]'
      style={{ transform: `translate(${x}px, ${y}px)` }}
    >
      <header className='flex h-8 shrink-0 items-center gap-1.5 border-b border-border/80 pr-1 pl-2'>
        <div
          {...attributes}
          {...listeners}
          className='flex min-w-0 flex-1 cursor-grab items-center gap-1.5 self-stretch active:cursor-grabbing'
        >
          <History className='size-3 text-primary' />
          <h2 className='text-[10px] font-semibold'>{t('history.title')}</h2>
        </div>
        <button
          type='button'
          aria-label={t('history.close')}
          className='grid size-6 place-items-center rounded-md text-muted-foreground hover:bg-foreground/[0.06] hover:text-foreground'
          onClick={() => setOpen(false)}
        >
          <X className='size-3.5' />
        </button>
      </header>
      <ScrollArea viewportClassName='max-h-72'>
        <ol className='grid gap-px p-1'>
          {states.map((label, step) => (
            <li key={step}>
              <button
                ref={step === history.current ? current : undefined}
                type='button'
                data-current={step === history.current}
                data-undone={step > history.current}
                className='w-full truncate rounded-md px-2 py-1 text-left text-[11px] hover:bg-foreground/[0.05] data-[current=true]:bg-accent data-[current=true]:font-medium data-[undone=true]:text-muted-foreground'
                onClick={() => step !== history.current && send(commands.goTo, step)}
              >
                {step === 0
                  ? t('history.opened')
                  : t(`history.steps.${label}`, { defaultValue: label })}
              </button>
            </li>
          ))}
        </ol>
      </ScrollArea>
    </section>
  )
}
