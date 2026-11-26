'use client'

import type { ReactNode } from 'react'

import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@koharu/ui/components/select'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'
import { cn } from '@koharu/ui/lib/utils'

/** One of `choices`, each shown by its label in the order listed: a settings field, or a bar's
 * compact select. A settings field opens in place, its chosen item over the trigger's; a compact
 * one drops on `side` of its trigger, as the type bar's preset lists do. A compact one has no label
 * beside it, so it names itself on hover, as a bar's buttons do, and, picked with the pointer,
 * leaves the keys to the canvas, as Photoshop's options bar does, so a tool's key that follows
 * switches the tool. */
export function Choice<Value extends string>({
  value,
  choices,
  label,
  size = 'default',
  side,
  disabled,
  className,
  onChange,
}: {
  /** Matching no choice shows nothing, as for a value the selection doesn't share. */
  value: Value | ''
  choices: Record<Value, ReactNode>
  /** Its accessible name. */
  label: string
  size?: 'default' | 'sm'
  side?: 'top' | 'bottom'
  disabled?: boolean
  className?: string
  onChange: (value: Value) => void
}) {
  const compact = size === 'sm'
  return (
    <Select<Value>
      value={value === '' ? null : value}
      items={choices}
      disabled={disabled}
      onValueChange={(next) => next && next !== value && onChange(next)}
    >
      {/* A settings field's label sits beside it already. */}
      <Tooltip disabled={!compact}>
        <TooltipTrigger
          render={
            <SelectTrigger
              size={size}
              aria-label={label}
              className={cn(!compact && 'w-full text-xs', className)}
            />
          }
        >
          <SelectValue />
        </TooltipTrigger>
        <TooltipContent>{label}</TooltipContent>
      </Tooltip>
      <SelectContent
        side={side}
        align={compact ? 'start' : 'center'}
        alignItemWithTrigger={!compact}
        finalFocus={compact ? (closed) => closed === 'keyboard' : undefined}
        // As wide as its trigger, wider only for a label that needs it.
        className={cn(compact && 'min-w-fit')}
      >
        {(Object.entries(choices) as [Value, ReactNode][]).map(([choice, name]) => (
          <SelectItem
            key={choice}
            value={choice}
            // The chosen one shows in the accent, its tick left out, which a list as narrow as
            // its trigger has no room for. Base UI marks it with a bare `data-selected`, which
            // shadcn's `data-selected:` variant, written for `="true"`, doesn't match.
            className={cn(
              compact &&
                'pr-2 data-[selected]:bg-accent data-[selected]:text-accent-foreground [&>span]:hidden!',
            )}
          >
            {name}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}
