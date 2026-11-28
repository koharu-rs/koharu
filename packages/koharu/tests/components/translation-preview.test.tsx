import { QueryClientProvider } from '@tanstack/react-query'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { expect, it, vi } from 'vitest'

import { TranslationPreview } from '@/components/editor/TranslationPreview'
import { queryClient } from '@/lib/queries'
import { commands, type Page } from '@koharu/ipc/protocol'

function preview() {
  const pages = Object.fromEntries(
    ['A', 'B'].map((id) => [
      id,
      {
        id,
        layers: ['Go', 'Wait'].map((ocr, i) => ({
          kind: 'block',
          id: id + i,
          source: { content: [{ insert: ocr }] },
          translation: { content: [{ insert: id === 'A' ? ['Đi', 'Đợi'][i] : '' }] },
        })),
      },
    ]),
  )
  vi.spyOn(commands, 'query').mockImplementation(async (q) =>
    q.query === 'get_page' ? (pages[q.page] as unknown as Page) : 'initial',
  )
  const edit = vi.spyOn(commands, 'edit').mockImplementation(async () => {
    throw Error('unexpected edit')
  })
  const close = vi.fn()
  render(
    <QueryClientProvider client={queryClient}>
      <TranslationPreview source='A' target='B' sourceNumber={1} targetNumber={2} close={close} />
    </QueryClientProvider>,
  )
  return { pages, edit, close }
}

it('copies only confirmed matches in one edit', async () => {
  const { edit, close } = preview()
  edit.mockResolvedValue({} as Awaited<ReturnType<typeof commands.edit>>)
  const boxes = await screen.findAllByRole('checkbox')
  const user = userEvent.setup()
  await user.click(boxes[0])
  await user.click(screen.getByRole('button', { name: 'Copy selected' }))
  await waitFor(() => expect(close).toHaveBeenCalledOnce())
  expect(edit).toHaveBeenCalledExactlyOnceWith(
    {
      label: 'set_text',
      ops: [{ op: 'set_text', layer: 'B1', text: 'Đợi' }],
    },
    'initial',
  )
})

it('keeps a translation filled after preview and asks for review', async () => {
  const { pages, edit } = preview()
  await screen.findAllByRole('checkbox')
  pages.B.layers[0].translation.content[0].insert = 'Manual change'
  vi.mocked(commands.query).mockImplementation(async (q) =>
    q.query === 'get_page' ? (pages[q.page] as unknown as Page) : 'updated',
  )
  edit.mockRejectedValue(new Error('project changed since preview'))
  await userEvent.click(screen.getByRole('button', { name: 'Copy selected' }))
  expect(await screen.findByText('The pages changed. Review the matches again.')).toBeVisible()
  expect(edit.mock.calls[0][1]).toBe('initial')
  expect(await screen.findAllByRole('checkbox')).toHaveLength(1)
})
