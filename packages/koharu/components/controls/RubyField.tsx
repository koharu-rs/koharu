'use client'

import { X } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { send } from '@/lib/queries'
import { commands, type RubyEdit, type Typed } from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'
import { Input } from '@koharu/ui/components/input'
import { Popover, PopoverContent, PopoverTrigger } from '@koharu/ui/components/popover'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'

/** The ruby over the characters selected while typing, or over the base the caret is in, in a row
 * of its own below the bar: one field with what it says selected, and Remove beside it, as Clip
 * Studio Paint's Reading setting and Pages' phonetic guide have them. The canvas holds it open and
 * sets the ruby as it's typed, where the page shows it over its base; a press there closes it,
 * which the interface never sees. Escape puts back what was there. */
export function RubyField({ typing }: { typing: Typed | null }) {
  const { t } = useTranslation()
  const [draft, setDraft] = useState('')
  const label = t('inspector.ruby')
  const remove = t('inspector.removeRuby')
  const base = typing?.base ?? null

  const annotate = (edit: RubyEdit) => send(commands.annotateTyping, edit)
  const set = (ruby: string) => {
    setDraft(ruby)
    annotate({ kind: 'set', ruby: ruby || null })
  }

  return (
    <Popover
      open={typing?.annotating ?? false}
      onOpenChange={(next, { reason }) => {
        if (next) {
          setDraft(base?.ruby ?? '')
          annotate({ kind: 'open' })
        } else annotate({ kind: reason === 'escape-key' ? 'cancel' : 'keep' })
      }}
    >
      <Tooltip>
        <TooltipTrigger
          render={
            <PopoverTrigger
              aria-label={label}
              disabled={!base}
              data-active={Boolean(base?.ruby)}
              className='grid size-6 place-items-center rounded-md border border-input bg-background text-muted-foreground outline-none hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-40 data-[active=true]:border-primary data-[active=true]:bg-primary data-[active=true]:text-primary-foreground data-[active=true]:hover:bg-primary/90'
            />
          }
        >
          {/* Three small characters over a large one. */}
          <svg
            aria-hidden='true'
            viewBox='0 0 24 24'
            fill='none'
            stroke='currentColor'
            strokeWidth={2}
            strokeLinecap='round'
            className='size-3.5'
          >
            <path d='M6 3h.01M12 3h.01M18 3h.01M12 8v3M5 11h14M8 11c1.5 5 5 8 10 10M16 11c-1.5 5-5 8-10 10' />
          </svg>
        </TooltipTrigger>
        <TooltipContent>{label}</TooltipContent>
      </Tooltip>
      {/* Clear of the bar, whose padding lies between it and the button. */}
      <PopoverContent
        aria-label={label}
        sideOffset={13}
        className='w-auto flex-row items-center gap-1 rounded-xl border-border bg-[var(--surface-floating)] p-1.5 shadow-[var(--shadow-toolrail)]'
      >
        <Input
          value={draft}
          aria-label={label}
          placeholder={label}
          className='h-6 w-44 rounded-md border-0 px-1.5 py-0 text-[12px] focus-visible:ring-0 dark:bg-transparent'
          onFocus={(event) => event.currentTarget.select()}
          onChange={(event) => set(event.currentTarget.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && !event.nativeEvent.isComposing) annotate({ kind: 'keep' })
          }}
        />
        <Tooltip>
          <TooltipTrigger
            render={
              <Button
                type='button'
                variant='ghost'
                size='icon-sm'
                aria-label={remove}
                disabled={!draft}
                className='size-6 rounded-md text-muted-foreground hover:text-foreground disabled:invisible'
                onClick={() => {
                  set('')
                  annotate({ kind: 'keep' })
                }}
              />
            }
          >
            <X className='size-3.5' strokeWidth={1.25} />
          </TooltipTrigger>
          <TooltipContent>{remove}</TooltipContent>
        </Tooltip>
      </PopoverContent>
    </Popover>
  )
}
