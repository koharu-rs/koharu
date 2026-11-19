'use client'

import { useQuery } from '@tanstack/react-query'
import { ArrowUpRight } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { call } from '@/lib/backend'
import { glossaryKey, refresh } from '@/lib/queries'
import { useKoharuStore } from '@/lib/store'
import { commands, type ProjectId, type TermLocation } from '@koharu/ipc/protocol'
import { Button } from '@koharu/ui/components/button'
import { Input } from '@koharu/ui/components/input'
import { ScrollArea } from '@koharu/ui/components/scroll-area'

export async function showEvidence({ page, layer }: Pick<TermLocation, 'page' | 'layer'>) {
  await call(commands.show, page)
  const { selectPages, selectLayers } = useKoharuStore.getState()
  selectPages([page])
  selectLayers([layer])
  await call(commands.select, [layer])
}

export function ReviewPanel({ project }: { project: ProjectId }) {
  const { t } = useTranslation()
  const { data } = useQuery({
    queryKey: [...glossaryKey, project],
    queryFn: () => call(commands.getGlossary),
  })
  const [edits, setEdits] = useState<Record<string, string>>({})
  const [busy, setBusy] = useState(false)
  if (!data || data.project !== project) return null
  const items = [
    ...data.suggestions.map((suggestion) => ({
      id: JSON.stringify(['alias', suggestion.source, suggestion.canonical]),
      title: t('review.alias', suggestion),
      reason: suggestion.reason,
      translation: suggestion.translation,
      locations: suggestion.locations,
      previous: '',
      resolve: (value: string | null) => call(commands.resolveAlias, project, suggestion, value),
    })),
    ...data.corrections.map((correction) => ({
      id: JSON.stringify([
        'translation',
        correction.location.layer,
        correction.previous,
        correction.translation,
      ]),
      title: t('review.correction'),
      reason: correction.reason,
      translation: correction.translation,
      locations: [correction.location],
      previous: correction.previous,
      resolve: (value: string | null) =>
        call(commands.resolveTranslation, project, correction, value),
    })),
  ]

  return (
    <ScrollArea className='h-full min-h-0 flex-1'>
      {!items.length && (
        <p role='status' className='p-3 text-xs text-muted-foreground'>
          {t('review.empty')}
        </p>
      )}
      {items.map((item) => {
        const draft = edits[item.id] ?? item.translation
        const resolve = async (value: string | null) => {
          setBusy(true)
          try {
            await item.resolve(value)
            await refresh(glossaryKey)
          } finally {
            setBusy(false)
          }
        }
        return (
          <section key={item.id} className='space-y-2 border-b p-3 text-xs'>
            <h3 className='font-semibold wrap-anywhere'>{item.title}</h3>
            {item.previous && <p className='wrap-anywhere'>{item.previous}</p>}
            <p className='wrap-anywhere text-muted-foreground'>{item.reason}</p>
            <ul className='space-y-1'>
              {item.locations.map((location) => (
                <li key={location.layer}>
                  <button
                    type='button'
                    className='flex w-full items-baseline gap-1 rounded text-start hover:text-primary'
                    onClick={() => void showEvidence(location).catch(() => undefined)}
                  >
                    <ArrowUpRight className='size-3 shrink-0' />
                    <span className='wrap-anywhere'>
                      {t('glossary.page', { page: location.number })} · {location.text}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
            <label className='flex items-center gap-2'>
              <span className='shrink-0'>{t('glossary.translation')}</span>
              <Input
                value={draft}
                onChange={(event) => setEdits({ ...edits, [item.id]: event.target.value })}
              />
            </label>
            <div className='flex gap-2'>
              <Button
                size='xs'
                disabled={busy || !draft.trim()}
                onClick={() => void resolve(draft.trim()).catch(() => undefined)}
              >
                {t('review.accept')}
              </Button>
              <Button
                size='xs'
                variant='outline'
                disabled={busy}
                onClick={() => void resolve(null).catch(() => undefined)}
              >
                {t('review.dismiss')}
              </Button>
            </div>
          </section>
        )
      })}
    </ScrollArea>
  )
}
