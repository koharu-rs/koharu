import { QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { TypeBar } from '@/components/editor/TypeBar'
import { fontsKey, pageKey, preferencesKey, queryClient } from '@/lib/queries'
import { setStyle } from '@/lib/scene'
import { useKoharuStore } from '@/lib/store'
import { commands, type Change, type Layer, type Op } from '@koharu/ipc/protocol'

const bounds = { x: 0, y: 0, width: 80, height: 40 }
const content = [{ insert: 'Hello' }]
const base = { visible: true, locked: false, opacity: 1, blend: 'normal', attributes: {} } as const

/** Selects a balloon's translation in a path and one outside balloons in a box, both fitted by
 * `fit`, then a growing box, point text, and a block not translated yet, and picks `choice` in
 * the type bar's font size menu. Returns the frame edits it sent. */
async function chooseSize(fit: 'none' | 'best', choice: string): Promise<() => Op[][]> {
  const layers: Layer[] = [
    {
      ...base,
      id: 'balloon',
      name: 'balloon',
      kind: 'block',
      source: { content },
      translation: { content, frame: { path: { path: 'M0 0H80V40H0Z', fit } } },
    },
    {
      ...base,
      id: 'sign',
      name: 'sign',
      kind: 'block',
      source: { content },
      translation: { content, frame: { box: { bounds, fit } } },
    },
    {
      ...base,
      id: 'note',
      name: 'note',
      kind: 'text',
      content,
      frame: { box: { bounds, fit: 'grow' } },
    },
    { ...base, id: 'caption', name: 'caption', kind: 'text', content, frame: 'point' },
    {
      ...base,
      id: 'untranslated',
      name: 'untranslated',
      kind: 'block',
      source: { content, frame: { box: { bounds } } },
    },
  ]
  queryClient.setQueryData(pageKey, { id: 'p', size: bounds, background: null, layers })
  queryClient.setQueryData(preferencesKey, { renderer: {} })
  queryClient.setQueryData(fontsKey, [])
  useKoharuStore.setState({ selectedLayers: layers.map(({ id }) => id), selectionBounds: bounds })
  const edit = vi.spyOn(commands, 'edit').mockResolvedValue({
    revision: '2',
    created: [],
    layers: [],
    pages: { added: [], removed: [], changed: [], reordered: false },
    history: { current: 1, steps: [] },
  } satisfies Change)
  render(
    <QueryClientProvider client={queryClient}>
      <TypeBar />
    </QueryClientProvider>,
  )
  fireEvent.click(screen.getByLabelText('Choose font size'))
  fireEvent.click(await screen.findByRole('menuitemradio', { name: choice }))
  return () =>
    edit.mock.calls.filter(([{ label }]) => label === 'update_layer').map(([{ ops }]) => ops)
}

const translated = (layer: string, frame: 'box' | 'path', fit: string): Op => ({
  op: 'update_layer',
  layer,
  patch: { translation: { frame: { [frame]: { fit } } } },
})

describe('type bar font size', () => {
  it('stops each fitted frame, a box or a path, fitting a chosen size, in one step', async () => {
    const frameEdits = await chooseSize('best', '24')
    await waitFor(() =>
      expect(frameEdits()).toEqual([
        [
          translated('balloon', 'path', 'none'),
          translated('sign', 'box', 'none'),
          translated('untranslated', 'box', 'none'),
        ],
      ]),
    )
  })

  it('fits each frame, a box or a path, for Auto, leaving texts without one alone', async () => {
    const frameEdits = await chooseSize('none', 'Auto')
    await waitFor(() =>
      expect(frameEdits()).toEqual([
        [
          translated('balloon', 'path', 'best'),
          translated('sign', 'box', 'best'),
          { op: 'update_layer', layer: 'note', patch: { frame: { box: { fit: 'best' } } } },
          translated('untranslated', 'box', 'best'),
        ],
      ]),
    )
  })
})

describe('restyling a text', () => {
  it('takes its characters own size off and keeps their ruby', async () => {
    const edit = vi.spyOn(commands, 'edit').mockResolvedValue({
      revision: '2',
      created: [],
      layers: [],
      pages: { added: [], removed: [], changed: [], reordered: false },
      history: { current: 1, steps: [] },
    } satisfies Change)
    const spans = [{ insert: 'sword', ruby: 'Excalibur', attributes: { size: 40, weight: 700 } }]
    const layer: Layer = { ...base, id: 't', name: 't', kind: 'text', content: spans }
    await setStyle([layer], { chars: { size: 24 } })
    expect(edit.mock.lastCall?.[0].ops[1]).toEqual({
      op: 'set_rich_text',
      layer: 't',
      spans: [{ insert: 'sword', ruby: 'Excalibur', attributes: { weight: 700 } }],
    })
  })
})
