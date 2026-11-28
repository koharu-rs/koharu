'use client'

import { useMutation, useQuery } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { translationMatches } from '@/lib/translation-matches'
import { commands, type PagePair } from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'
import { Checkbox } from '@koharu/ui/components/checkbox'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@koharu/ui/components/dialog'

export function TranslationPreview({
  source,
  target,
  sourceNumber,
  targetNumber,
  close,
}: {
  source: string
  target: string
  sourceNumber: number
  targetNumber: number
  close: () => void
}) {
  const { t } = useTranslation()
  const preview = useQuery({
    queryKey: ['translation-preview', source, target],
    gcTime: 0,
    queryFn: async () => {
      const pair = (await commands.query({ query: 'get_page_pair', source, target })) as PagePair
      return { revision: pair.revision, matches: translationMatches(pair.source, pair.target) }
    },
  })
  const [excluded, setExcluded] = useState<string[]>([])
  const matches = preview.data?.matches ?? []
  const selected = matches.filter((match) => !excluded.includes(match.target))
  const apply = useMutation({
    mutationFn: async () => {
      if (!preview.data || !selected.length) return
      const { revision } = preview.data
      try {
        await commands.edit(
          {
            label: 'set_text',
            ops: selected.map(({ target: layer, text }) => ({ op: 'set_text', layer, text })),
          },
          revision,
        )
      } catch (error) {
        const refreshed = await preview.refetch()
        if (refreshed.data && refreshed.data.revision !== revision)
          throw new Error(t('matching.changed'))
        throw error
      }
    },
    onSuccess: close,
  })
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !apply.isPending) close()
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t('matching.title')}</DialogTitle>
          <DialogDescription>
            {t('matching.direction', { from: sourceNumber, to: targetNumber })}
          </DialogDescription>
        </DialogHeader>
        {preview.isError ? (
          <p role='alert'>{t('errors.actionFailed')}</p>
        ) : !preview.data ? (
          <p>{t('common.loading')}</p>
        ) : !matches.length ? (
          <p>{t('matching.empty')}</p>
        ) : (
          <div className='grid max-h-[50vh] gap-2 overflow-auto'>
            {matches.map(({ target: id, ocr, text }) => (
              <label key={id} className='flex items-start gap-2'>
                <Checkbox
                  checked={!excluded.includes(id)}
                  disabled={apply.isPending}
                  onCheckedChange={(checked) =>
                    setExcluded((old) =>
                      checked ? old.filter((item) => item !== id) : [...old, id],
                    )
                  }
                />
                <span className='min-w-0 break-words whitespace-pre-wrap'>
                  <span className='block text-muted-foreground'>{ocr}</span>
                  {text}
                </span>
              </label>
            ))}
          </div>
        )}
        {apply.error && <p role='alert'>{apply.error.message}</p>}
        <div className='flex justify-end gap-2'>
          <Button variant='outline' onClick={close} disabled={apply.isPending}>
            {t('common.cancel')}
          </Button>
          <Button
            onClick={() => apply.mutate()}
            disabled={!selected.length || apply.isPending || preview.isError}
          >
            {t('matching.copy')}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  )
}
