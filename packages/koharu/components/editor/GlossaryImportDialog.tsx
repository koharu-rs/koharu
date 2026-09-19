'use client'

import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import type { GlossaryImportPreview, GlossaryImportStrategy } from '@koharu/bridge/protocol'
import { Button } from '@koharu/ui/components/button'
import { Checkbox } from '@koharu/ui/components/checkbox'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@koharu/ui/components/dialog'

interface GlossaryImportDialogProps {
  open: boolean
  preview: GlossaryImportPreview | null
  pending: boolean
  onOpenChange: (open: boolean) => void
  onApply: (strategy: GlossaryImportStrategy, confirmLanguageMismatch: boolean) => void
}

export function GlossaryImportDialog({
  open,
  preview,
  pending,
  onOpenChange,
  onApply,
}: GlossaryImportDialogProps) {
  const { t } = useTranslation()
  const [strategy, setStrategy] = useState<GlossaryImportStrategy>('keep_existing')
  const [confirmed, setConfirmed] = useState(false)
  const mismatched = (preview?.languageMismatches.length ?? 0) > 0

  useEffect(() => {
    if (!open) return
    setStrategy('keep_existing')
    setConfirmed(false)
  }, [open, preview])

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className='max-w-md gap-3'>
        <DialogHeader>
          <DialogTitle>{t('glossary.importTitle')}</DialogTitle>
          <DialogDescription>{t('glossary.importDescription')}</DialogDescription>
        </DialogHeader>
        {preview ? (
          <>
            <div className='grid grid-cols-3 gap-2' aria-label={t('glossary.importSummary')}>
              <ImportCount
                value={preview.added}
                label={t('glossary.importAdded', { count: preview.added })}
              />
              <ImportCount
                value={preview.conflicting}
                label={t('glossary.importConflicts', { count: preview.conflicting })}
              />
              <ImportCount
                value={preview.identical}
                label={t('glossary.importIdentical', { count: preview.identical })}
              />
            </div>
            <fieldset className='space-y-2'>
              <legend className='mb-1 text-[11px] font-medium'>
                {t('glossary.importStrategy')}
              </legend>
              <label className='flex items-start gap-2 rounded-lg border p-2 text-[11px]'>
                <input
                  type='radio'
                  aria-label={t('glossary.keepExisting')}
                  name='glossary-import-strategy'
                  value='keep_existing'
                  checked={strategy === 'keep_existing'}
                  disabled={pending}
                  onChange={() => setStrategy('keep_existing')}
                />
                <span>
                  <span className='block font-medium'>{t('glossary.keepExisting')}</span>
                  <span className='text-muted-foreground'>
                    {t('glossary.keepExistingDescription')}
                  </span>
                </span>
              </label>
              <label className='flex items-start gap-2 rounded-lg border p-2 text-[11px]'>
                <input
                  type='radio'
                  aria-label={t('glossary.replaceExisting')}
                  name='glossary-import-strategy'
                  value='replace_existing'
                  checked={strategy === 'replace_existing'}
                  disabled={pending}
                  onChange={() => setStrategy('replace_existing')}
                />
                <span>
                  <span className='block font-medium'>{t('glossary.replaceExisting')}</span>
                  <span className='text-muted-foreground'>
                    {t('glossary.replaceExistingDescription')}
                  </span>
                </span>
              </label>
            </fieldset>
            {mismatched ? (
              <div className='rounded-lg border border-amber-500/30 bg-amber-500/5 p-2.5 text-[11px]'>
                <p className='font-medium text-amber-700 dark:text-amber-300'>
                  {t('glossary.languageMismatch')}
                </p>
                <ul className='mt-1 space-y-0.5 text-muted-foreground'>
                  {preview.languageMismatches.map((mismatch) => (
                    <li key={mismatch.field}>
                      {t(`glossary.languageField.${mismatch.field}`)}: {mismatch.current ?? '—'} →{' '}
                      {mismatch.imported ?? '—'}
                    </li>
                  ))}
                </ul>
                <label className='mt-2 flex items-center gap-2 text-foreground'>
                  <Checkbox
                    checked={confirmed}
                    disabled={pending}
                    onCheckedChange={(checked) => setConfirmed(checked === true)}
                  />
                  {t('glossary.confirmLanguageMismatch')}
                </label>
              </div>
            ) : null}
          </>
        ) : null}
        <DialogFooter>
          <Button variant='outline' disabled={pending} onClick={() => onOpenChange(false)}>
            {t('common.cancel')}
          </Button>
          <Button
            disabled={!preview || pending || (mismatched && !confirmed)}
            onClick={() => onApply(strategy, confirmed)}
          >
            {t('glossary.importAction')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function ImportCount({ value, label }: { value: number; label: string }) {
  return (
    <div className='rounded-lg bg-muted/60 p-2 text-center'>
      <span className='block text-base font-semibold tabular-nums'>{value}</span>
      <span className='text-[10px] text-muted-foreground'>{label}</span>
    </div>
  )
}
