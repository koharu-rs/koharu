'use client'

import { Input } from '@koharu/ui/components/input'
import {
  NumberField as NumberFieldRoot,
  NumberFieldGroup,
  NumberFieldInput,
} from '@koharu/ui/components/number-field'

export function PreferencePage({
  title,
  description,
  children,
}: {
  title: string
  description: string
  children: React.ReactNode
}) {
  return (
    <div className='max-w-3xl'>
      <header className='mb-8'>
        <h2 className='text-[20px] font-semibold tracking-[-0.02em]'>{title}</h2>
        <p className='mt-1 max-w-xl text-[12px] leading-5 text-muted-foreground'>{description}</p>
      </header>
      <div className='grid gap-8'>{children}</div>
    </div>
  )
}

export function PreferenceSection({
  title,
  description,
  children,
}: {
  title: string
  description?: string
  children: React.ReactNode
}) {
  return (
    <section>
      <div className='mb-3'>
        <h3 className='text-[12px] font-semibold'>{title}</h3>
        {description && (
          <p className='mt-0.5 text-[11px] leading-4 text-muted-foreground'>{description}</p>
        )}
      </div>
      <div className='divide-y divide-border rounded-xl border border-border/80 bg-[var(--surface-panel)] px-5'>
        {children}
      </div>
    </section>
  )
}

export function PreferenceRow({
  title,
  description,
  children,
  align = 'center',
}: {
  title: string
  description?: string
  children: React.ReactNode
  align?: 'center' | 'start'
}) {
  return (
    <div
      className={`grid grid-cols-[minmax(150px,0.9fr)_minmax(220px,1.1fr)] gap-6 py-3.5 ${align === 'start' ? 'items-start' : 'items-center'}`}
    >
      <div>
        <p className='text-[12px] font-medium'>{title}</p>
        {description && (
          <p className='mt-0.5 text-[10px] leading-4 text-muted-foreground'>{description}</p>
        )}
      </div>
      <div className='min-w-0'>{children}</div>
    </div>
  )
}

export function TextField({
  label,
  value,
  type = 'text',
  onChange,
}: {
  label: string
  value: string
  type?: 'text' | 'url'
  onChange: (value: string) => void
}) {
  return (
    <label className='grid gap-1 text-[10px] text-muted-foreground'>
      {label}
      <Input
        type={type}
        value={value}
        className='h-8 text-[12px] text-foreground'
        onChange={(event) => onChange(event.currentTarget.value)}
      />
    </label>
  )
}

/** A number the field clamps to `min` and `max` as it's typed, or nothing. */
export function NumberField({
  label,
  value,
  min,
  max,
  step,
  integer = false,
  placeholder,
  onChange,
}: {
  label: string
  value: number | null
  min?: number
  max?: number
  step?: number
  /** Whole numbers only. */
  integer?: boolean
  placeholder?: string
  onChange: (value: number | null) => void
}) {
  // The label takes the room a wrapped neighbour's label adds, so inputs side by side line up.
  return (
    <label className='grid grid-rows-[1fr_auto] gap-1 text-[10px] text-muted-foreground'>
      {label}
      <NumberFieldRoot
        value={value}
        min={min}
        max={max}
        step={step}
        format={integer ? { maximumFractionDigits: 0, useGrouping: false } : undefined}
        onValueChange={onChange}
      >
        <NumberFieldGroup className='h-8 rounded-lg focus-within:ring-3 focus-within:ring-ring/50'>
          <NumberFieldInput
            placeholder={placeholder}
            className='px-2.5 text-left text-[12px] text-foreground placeholder:text-muted-foreground'
          />
        </NumberFieldGroup>
      </NumberFieldRoot>
    </label>
  )
}
