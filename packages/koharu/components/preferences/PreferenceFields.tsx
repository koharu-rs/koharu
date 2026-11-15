'use client'

import { useId } from 'react'
import { useTranslation } from 'react-i18next'

import { Input } from '@koharu/ui/components/input'

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

/**
 * `min` and `max` on a number input are validation hints the browser never
 * enforces on typed text, so bounds are applied here before the value reaches
 * the stored configuration.
 */
function parseBounded(raw: string, min?: number, max?: number) {
  if (raw === '') return null
  const value = Number(raw)
  if (!Number.isFinite(value)) return null
  if (min !== undefined && value < min) return min
  if (max !== undefined && value > max) return max
  return value
}

export function NumberField({
  label,
  value,
  min,
  max,
  step,
  onChange,
}: {
  label: string
  value: number | null
  min?: number
  max?: number
  step?: number
  onChange: (value: number | null) => void
}) {
  const { t } = useTranslation()
  const id = useId()
  // The label takes the room a wrapped neighbour's label adds, so inputs side by side line up.
  return (
    <label className='grid grid-rows-[1fr_auto] gap-1 text-[10px] text-muted-foreground'>
      {label}
      <Input
        id={id}
        type='number'
        value={value ?? ''}
        min={min}
        max={max}
        step={step}
        placeholder={t('model.default')}
        className='h-8 text-[12px] text-foreground'
        onChange={(event) => onChange(parseBounded(event.currentTarget.value, min, max))}
      />
    </label>
  )
}
