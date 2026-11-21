'use client'

import { ChevronLeft } from 'lucide-react'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import { Button } from '@koharu/ui/components/button'

/** A page a drill-down menu opened, as Claude's and Codex's menus do: a heading that goes back,
 * then its own rows. */
export function MenuPanel({
  title,
  backDisabled = false,
  onBack,
  children,
}: {
  title: string
  backDisabled?: boolean
  onBack: () => void
  children: ReactNode
}) {
  const { t } = useTranslation()
  return (
    <div className='min-w-0 overflow-hidden'>
      <div className='mb-1 flex h-7 items-center border-b border-border/60 px-0.5 pb-1'>
        <Button
          type='button'
          variant='ghost'
          size='icon-xs'
          aria-label={t('common.back')}
          className='rounded-md text-muted-foreground hover:bg-primary/10 hover:text-foreground'
          disabled={backDisabled}
          onClick={onBack}
        >
          <ChevronLeft className='size-3.5' />
        </Button>
        <span className='ml-1 min-w-0 flex-1 truncate text-[11px] font-medium'>{title}</span>
      </div>
      {children}
    </div>
  )
}
