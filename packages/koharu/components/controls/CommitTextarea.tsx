'use client'

import { useEffect, useRef, useState, type ComponentProps } from 'react'

import { Textarea } from '@koharu/ui/components/textarea'

type HistoryDirection = 'undo' | 'redo'

type CommitTextareaProps = Omit<ComponentProps<typeof Textarea>, 'value' | 'onChange'> & {
  value: string
  delay?: number
  onCommit: (value: string) => void | Promise<void>
  onHistoryNavigate?: (direction: HistoryDirection) => void | Promise<void>
}

export function CommitTextarea({
  value,
  delay = 360,
  onCommit,
  onHistoryNavigate,
  ...props
}: CommitTextareaProps) {
  const [draft, setDraft] = useState(value)
  const timer = useRef<number | null>(null)
  const composing = useRef(false)
  const external = useRef(value)

  useEffect(() => {
    external.current = value
    if (!composing.current && timer.current === null) setDraft(value)
  }, [value])

  useEffect(
    () => () => {
      if (timer.current !== null) window.clearTimeout(timer.current)
    },
    [],
  )

  const commit = async (next: string) => {
    if (timer.current !== null) window.clearTimeout(timer.current)
    timer.current = null
    if (next !== external.current) await onCommit(next)
  }

  const schedule = (next: string) => {
    if (timer.current !== null) window.clearTimeout(timer.current)
    timer.current = window.setTimeout(() => void commit(next).catch(() => undefined), delay)
  }

  return (
    <Textarea
      {...props}
      value={draft}
      onChange={(event) => {
        const next = event.currentTarget.value
        setDraft(next)
        if (!composing.current) schedule(next)
      }}
      onCompositionStart={() => {
        composing.current = true
      }}
      onCompositionEnd={(event) => {
        composing.current = false
        const next = event.currentTarget.value
        setDraft(next)
        schedule(next)
      }}
      onKeyDown={(event) => {
        if (!onHistoryNavigate || composing.current || (!event.ctrlKey && !event.metaKey)) return
        const key = event.key.toLowerCase()
        if (key !== 'z' && key !== 'y') return
        event.preventDefault()
        event.stopPropagation()
        const direction = key === 'y' || event.shiftKey ? 'redo' : 'undo'
        void commit(draft)
          .then(() => onHistoryNavigate(direction))
          .catch(() => undefined)
      }}
      onBlur={() => void commit(draft).catch(() => undefined)}
    />
  )
}
