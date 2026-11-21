'use client'

import { useQuery } from '@tanstack/react-query'
import { getVersion } from '@tauri-apps/api/app'
import Image from 'next/image'
import { useTranslation } from 'react-i18next'

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@koharu/ui/components/dialog'

const author = 'Mayo Takanashi'

export function AboutDialog({
  open,
  onOpenChange,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()
  // `null` when the version can't be read, which About says without a toast.
  const version = useQuery({
    queryKey: ['version'],
    queryFn: () => getVersion().catch(() => null),
    enabled: open,
  }).data

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className='max-w-xs gap-4 p-5'>
        <div className='flex items-center gap-3'>
          <div className='grid size-12 shrink-0 place-items-center rounded-xl bg-primary/10'>
            <Image src='/icon.png' alt='' width={30} height={30} priority />
          </div>
          <DialogHeader className='min-w-0 gap-1'>
            <DialogTitle className='text-[15px]'>Koharu</DialogTitle>
            <DialogDescription className='text-[11px]'>{t('about.description')}</DialogDescription>
          </DialogHeader>
        </div>

        <dl className='grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 border-t border-border/70 pt-3 text-[11px]'>
          <dt className='text-muted-foreground'>{t('about.version')}</dt>
          <dd className='text-right font-medium'>
            {version === null ? t('about.unavailable') : (version ?? t('common.loading'))}
          </dd>
          <dt className='text-muted-foreground'>{t('about.author')}</dt>
          <dd className='text-right font-medium'>{author}</dd>
        </dl>
      </DialogContent>
    </Dialog>
  )
}
