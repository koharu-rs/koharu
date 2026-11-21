'use client'

import { Search, X } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { ComboboxInput } from '@koharu/ui/components/combobox'
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from '@koharu/ui/components/input-group'
import { cn } from '@koharu/ui/lib/utils'

/** Every search line's look: a quiet well, its ring left to the field it sits in. */
const searchLine =
  'h-7 rounded-md border-0 bg-muted/50 shadow-none focus-within:bg-background has-[[data-slot=input-group-control]:focus-visible]:ring-0 dark:bg-muted/50'

/** A picker's search line: the combobox's query, which filters its list as it's typed, and a
 * button that clears it. */
export function PickerSearch({
  query,
  label,
  onClear,
}: {
  query: string
  label: string
  onClear: () => void
}) {
  return (
    <ComboboxInput
      autoFocus
      showTrigger={false}
      aria-label={label}
      placeholder={label}
      className={cn(searchLine, '[&_input]:h-7 [&_input]:px-0 [&_input]:text-[11px]')}
    >
      <SearchAddons query={query} onClear={onClear} />
    </ComboboxInput>
  )
}

/** A list's search line outside a picker, as a picker's looks: `query` filters the list as it's
 * typed, and a button clears it. */
export function ListSearch({
  query,
  label,
  className,
  onChange,
}: {
  query: string
  label: string
  className?: string
  onChange: (query: string) => void
}) {
  return (
    <InputGroup className={cn(searchLine, className)}>
      <InputGroupInput
        type='search'
        value={query}
        aria-label={label}
        placeholder={label}
        autoComplete='off'
        spellCheck={false}
        className='h-7 px-0 text-[11px] [&::-webkit-search-cancel-button]:hidden'
        onChange={(event) => onChange(event.target.value)}
      />
      <SearchAddons query={query} onClear={() => onChange('')} />
    </InputGroup>
  )
}

function SearchAddons({ query, onClear }: { query: string; onClear: () => void }) {
  const { t } = useTranslation()
  return (
    <>
      <InputGroupAddon className='pr-1 pl-2'>
        <Search className='size-3' />
      </InputGroupAddon>
      {query && (
        <InputGroupAddon align='inline-end' className='pr-1'>
          <InputGroupButton size='icon-xs' aria-label={t('common.clearSearch')} onClick={onClear}>
            <X />
          </InputGroupButton>
        </InputGroupAddon>
      )}
    </>
  )
}
