'use client'

import { useEffect, useRef, useState, type ComponentProps } from 'react'
import { useDebouncedCallback } from 'use-debounce'

import { Textarea } from '@koharu/ui/components/textarea'

type CommitTextareaProps = Omit<ComponentProps<typeof Textarea>, 'value' | 'onChange'> & {
  value: string
  delay?: number
  onCommit: (value: string) => void
}

/** A textarea that commits its text once typing pauses, or as it loses the focus or goes away, and
 * never mid-composition; the value it's given replaces its text only while nothing waits to
 * commit. */
export function CommitTextarea({ value, delay = 360, onCommit, ...props }: CommitTextareaProps) {
  const [draft, setDraft] = useState(value)
  const composing = useRef(false)
  const commit = useDebouncedCallback(
    (next: string) => {
      if (next !== value) onCommit(next)
    },
    delay,
    { flushOnExit: true },
  )

  useEffect(() => {
    if (!composing.current && !commit.isPending()) setDraft(value)
  }, [commit, value])

  return (
    <Textarea
      {...props}
      value={draft}
      onChange={(event) => {
        const next = event.currentTarget.value
        setDraft(next)
        if (!composing.current) commit(next)
      }}
      onCompositionStart={() => {
        composing.current = true
      }}
      onCompositionEnd={(event) => {
        composing.current = false
        const next = event.currentTarget.value
        setDraft(next)
        commit(next)
      }}
      onBlur={() => {
        commit(draft)
        commit.flush()
      }}
    />
  )
}
