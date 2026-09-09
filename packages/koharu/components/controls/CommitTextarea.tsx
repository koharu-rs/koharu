'use client'

import { useEffect, useRef, useState, type ComponentProps } from 'react'

import { Textarea } from '@koharu/ui/components/textarea'

type CommitTextareaProps = Omit<ComponentProps<typeof Textarea>, 'value' | 'onChange'> & {
  value: string
  delay?: number
  onCommit: (value: string) => void
}

export function CommitTextarea({ value, delay = 360, onCommit, ...props }: CommitTextareaProps) {
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

  const commit = (next: string) => {
    if (timer.current !== null) window.clearTimeout(timer.current)
    timer.current = null
    if (next !== external.current) onCommit(next)
  }

  const schedule = (next: string) => {
    if (timer.current !== null) window.clearTimeout(timer.current)
    timer.current = window.setTimeout(() => commit(next), delay)
  }

  return (
    <Textarea
      {...props}
      data-history-undo='true'
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
      onBlur={() => commit(draft)}
    />
  )
}
