import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { ProjectGlossaryDialog } from '@/components/app/ProjectGlossaryDialog'
import { glossaryTranslationBatches } from '@/lib/glossary'
import { commands, type GlossaryCandidate } from '@koharu/bridge/protocol'

const candidate = (source: string, suggested_target = ''): GlossaryCandidate => ({
  source,
  suggested_target,
  category: 'person',
  occurrences: 2,
  page_count: 2,
})

describe('AI glossary translation', () => {
  it('bounds batches by count and UTF-8 bytes and preserves filled terms', () => {
    expect(
      glossaryTranslationBatches(
        Array.from({ length: 25 }, (_, index) => candidate(`term${index}`)),
      ).map((batch) => batch.length),
    ).toEqual([24, 1])
    expect(
      glossaryTranslationBatches(Array.from({ length: 5 }, () => candidate('あ'.repeat(341)))).map(
        (batch) => batch.length,
      ),
    ).toEqual([4, 1])
    expect(glossaryTranslationBatches([candidate('高橋', '高桥'), candidate('田中')])).toEqual([
      [{ index: 1, source: '田中' }],
    ])
  })

  it('fills blank candidate translations without confirming or overwriting existing translations', async () => {
    vi.spyOn(commands, 'getProjectGlossary').mockResolvedValue({
      project: 'project',
      glossary: {
        entries: [],
        ignored: [],
        candidates: [candidate('高橋', '高桥'), candidate('田中')],
      },
    })
    const translate = vi
      .spyOn(commands, 'suggestGlossaryTranslations')
      .mockResolvedValue(['田中同学'])
    const save = vi.spyOn(commands, 'saveProjectGlossary')
    render(<ProjectGlossaryDialog open onOpenChange={vi.fn()} />)
    fireEvent.click(await screen.findByRole('button', { name: 'AI fill missing translations' }))
    await waitFor(() => expect(screen.getByLabelText('Translation 2')).toHaveValue('田中同学'))
    expect(screen.getByLabelText('Translation 1')).toHaveValue('高桥')
    expect(screen.getByRole('button', { name: 'Confirmed (0)' })).toBeInTheDocument()
    expect(translate).toHaveBeenCalledWith('project', ['田中'], [])
    expect(save).not.toHaveBeenCalled()
  })

  it('stops issuing new batches and keeps in-flight suggestions', async () => {
    vi.spyOn(commands, 'getProjectGlossary').mockResolvedValue({
      project: 'project',
      glossary: {
        entries: [],
        ignored: [],
        candidates: Array.from({ length: 73 }, (_, index) => candidate(`term${index}`)),
      },
    })
    const pending: ((value: string[]) => void)[] = []
    const translate = vi
      .spyOn(commands, 'suggestGlossaryTranslations')
      .mockImplementation(
        () =>
          new Promise((resolve) => {
            pending.push(resolve)
          }),
      )
    render(<ProjectGlossaryDialog open onOpenChange={vi.fn()} />)
    fireEvent.click(await screen.findByRole('button', { name: 'AI fill missing translations' }))
    await waitFor(() => expect(translate).toHaveBeenCalledTimes(3))
    fireEvent.click(screen.getByRole('button', { name: 'Stop after this batch' }))
    pending[0](Array.from({ length: 24 }, (_, index) => `a${index}`))
    pending[1](Array.from({ length: 24 }, (_, index) => `b${index}`))
    pending[2](Array.from({ length: 24 }, (_, index) => `c${index}`))
    await waitFor(() => expect(screen.getByLabelText('Translation 25')).toHaveValue('b0'))
    expect(translate).toHaveBeenCalledTimes(3)
    expect(screen.getByLabelText('Translation 1')).toHaveValue('a0')
    expect(screen.getByLabelText('Translation 24')).toHaveValue('a23')
    expect(screen.getByLabelText('Translation 48')).toHaveValue('b23')
    expect(screen.getByLabelText('Translation 49')).toHaveValue('c0')
  })
})
