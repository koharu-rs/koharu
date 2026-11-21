'use client'

import { NumberField as NumberFieldPrimitive } from '@base-ui/react/number-field'
import * as React from 'react'

import { cn } from '@koharu/ui/lib/utils'

const NumberField = NumberFieldPrimitive.Root

function NumberFieldGroup({ className, ...props }: NumberFieldPrimitive.Group.Props) {
  return (
    <NumberFieldPrimitive.Group
      data-slot='number-field-group'
      className={cn(
        'flex h-6 w-full min-w-0 items-center overflow-hidden rounded-md border border-input bg-transparent transition-colors focus-within:border-ring focus-within:ring-2 focus-within:ring-ring/30 dark:bg-input/30 data-disabled:cursor-not-allowed data-disabled:opacity-50',
        className,
      )}
      {...props}
    />
  )
}

/** Enter takes a typed value and leaves the field, as a number field in Photoshop's bars and
 * panels does; Base UI commits a typed value only when the field loses focus. */
function NumberFieldInput({ className, onKeyDown, ...props }: NumberFieldPrimitive.Input.Props) {
  return (
    <NumberFieldPrimitive.Input
      data-slot='number-field-input'
      className={cn(
        'h-full min-w-0 flex-1 bg-transparent px-1 text-center text-[11px] tabular-nums outline-none selection:bg-primary selection:text-primary-foreground',
        className,
      )}
      onKeyDown={(event) => {
        onKeyDown?.(event)
        if (event.key === 'Enter' && !event.defaultPrevented && !event.nativeEvent.isComposing) {
          event.currentTarget.blur()
        }
      }}
      {...props}
    />
  )
}

function NumberFieldDecrement({ className, ...props }: NumberFieldPrimitive.Decrement.Props) {
  return (
    <NumberFieldPrimitive.Decrement
      data-slot='number-field-decrement'
      className={cn(
        'grid size-6 shrink-0 place-items-center border-r border-input text-muted-foreground transition-colors outline-none hover:bg-muted hover:text-foreground focus-visible:bg-muted focus-visible:text-foreground disabled:pointer-events-none disabled:opacity-40 [&_svg:not([class*="size-"])]:size-3',
        className,
      )}
      {...props}
    />
  )
}

function NumberFieldIncrement({ className, ...props }: NumberFieldPrimitive.Increment.Props) {
  return (
    <NumberFieldPrimitive.Increment
      data-slot='number-field-increment'
      className={cn(
        'grid size-6 shrink-0 place-items-center border-l border-input text-muted-foreground transition-colors outline-none hover:bg-muted hover:text-foreground focus-visible:bg-muted focus-visible:text-foreground disabled:pointer-events-none disabled:opacity-40 [&_svg:not([class*="size-"])]:size-3',
        className,
      )}
      {...props}
    />
  )
}

export {
  NumberField,
  NumberFieldDecrement,
  NumberFieldGroup,
  NumberFieldIncrement,
  NumberFieldInput,
}
