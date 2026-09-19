import { QueryClientProvider } from '@tanstack/react-query'
import { act, fireEvent, render as testingRender, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { ReactNode } from 'react'
import { describe, expect, it, vi } from 'vitest'

import { GlossaryPanel } from '@/components/editor/GlossaryPanel'
import { RightSidebar } from '@/components/editor/RightSidebar'
import { glossaryKey, projectKey, queryClient } from '@/lib/queries'
import { useKoharuStore } from '@/lib/store'
import { commands } from '@koharu/bridge'
import type { GlossaryEntryView, GlossaryView, ProjectInfo } from '@koharu/bridge/protocol'
import { toast } from '@koharu/ui/components/toast'
import { TooltipProvider } from '@koharu/ui/components/tooltip'

const project: ProjectInfo = {
  name: 'Book',
  revision: 7,
  active_page: null,
  can_undo: true,
  can_redo: false,
}

const entries: GlossaryEntryView[] = [
  {
    revision: 7,
    id: 'haruka',
    source: '春香',
    translation: 'Haruka',
    kind: 'person',
    enabled: true,
    confidence: 0.98,
    occurrenceCount: 4,
    examples: ['春香さんは走った'],
    sourceOrigin: 'detected',
    translationOrigin: 'user',
    presentInLastScan: true,
  },
  {
    revision: 7,
    id: 'academy',
    source: '月影学園',
    translation: null,
    kind: 'organization',
    enabled: true,
    confidence: 0.82,
    occurrenceCount: 2,
    examples: [],
    sourceOrigin: 'detected',
    translationOrigin: null,
    presentInLastScan: false,
  },
  {
    revision: 7,
    id: 'blade',
    source: '星斬り',
    translation: null,
    kind: 'ability',
    enabled: false,
    confidence: null,
    occurrenceCount: 0,
    examples: [],
    sourceOrigin: 'user',
    translationOrigin: null,
    presentInLastScan: true,
  },
]

function glossary(overrides: Partial<GlossaryView> = {}): GlossaryView {
  return {
    revision: 7,
    enabled: true,
    stale: false,
    sourceLanguage: 'ja',
    targetLanguage: 'en',
    savedSourceFingerprint: 'saved',
    currentSourceFingerprint: 'saved',
    entries,
    ...overrides,
  }
}

function install(data: GlossaryView = glossary()) {
  queryClient.setQueryData(projectKey, project)
  queryClient.setQueryData(glossaryKey, data)
  vi.spyOn(commands, 'getProject').mockResolvedValue(project)
  vi.spyOn(commands, 'getGlossary').mockImplementation(
    async () => queryClient.getQueryData<GlossaryView>(glossaryKey) ?? data,
  )
}

function render(ui: ReactNode) {
  return testingRender(
    <QueryClientProvider client={queryClient}>
      <TooltipProvider>{ui}</TooltipProvider>
    </QueryClientProvider>,
  )
}

async function openActions() {
  const trigger = screen.getByRole('button', { name: 'Glossary actions' })
  fireEvent.click(trigger)
  await waitFor(() => expect(trigger).toHaveAttribute('aria-expanded', 'true'))
}

describe('project glossary panel', () => {
  it('disables the glossary tab when no project is open', () => {
    queryClient.setQueryData(projectKey, null)
    vi.spyOn(commands, 'getProject').mockResolvedValue(null)

    render(<RightSidebar />)

    expect(screen.getByRole('tab', { name: 'Glossary' })).toHaveAttribute('aria-disabled', 'true')
  })

  it('offers a first scan and changes to rescan when the glossary is stale', async () => {
    const user = userEvent.setup()
    install(glossary({ savedSourceFingerprint: null, entries: [] }))
    const scan = vi.spyOn(commands, 'scanGlossary').mockResolvedValue('scan-job')
    const view = render(<GlossaryPanel />)

    expect(screen.getByText('Not scanned')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Scan glossary' }))
    await waitFor(() => expect(scan).toHaveBeenCalledOnce())

    queryClient.setQueryData(glossaryKey, glossary({ stale: true }))
    view.rerender(
      <QueryClientProvider client={queryClient}>
        <TooltipProvider>
          <GlossaryPanel />
        </TooltipProvider>
      </QueryClientProvider>,
    )
    expect(await screen.findByText('Needs rescan')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Rescan glossary' })).toBeInTheDocument()
  })

  it.each([
    ['glossary_scan', { kind: 'preparing_ocr' }, 'Preparing OCR'],
    ['glossary_scan', { kind: 'extracting_terms' }, 'Extracting terms'],
    ['glossary_translation', { kind: 'translating_terms' }, 'Translating terms'],
  ] as const)('shows the %s progress phase', (kind, phase, label) => {
    install()
    useKoharuStore.setState({
      jobs: {
        glossary: {
          id: 'glossary',
          kind,
          phase,
          state: 'running',
          completed: 1,
          total: 4,
          page: null,
          error: null,
        },
      },
    })

    render(<GlossaryPanel />)

    expect(screen.getByRole('status')).toHaveTextContent(label)
    expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow', '25')
    expect(screen.getByRole('button', { name: 'Rescan glossary' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Glossary actions' })).toBeDisabled()
  })

  it('shows waiting for confirmation as an idle post-scan state', () => {
    install()
    render(<GlossaryPanel />)

    expect(screen.getByText('Waiting for confirmation')).toBeInTheDocument()
    expect(screen.queryByRole('progressbar')).not.toBeInTheDocument()
  })

  it('serializes inline edits and sends the complete generated patch shape', async () => {
    install()
    const first = Promise.withResolvers<GlossaryView>()
    const second = Promise.withResolvers<GlossaryView>()
    const update = vi
      .spyOn(commands, 'updateGlossaryEntry')
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise)
    render(<GlossaryPanel />)
    const source = screen.getByRole('textbox', { name: 'Source term 春香' })

    fireEvent.change(source, { target: { value: '春香 first' } })
    fireEvent.blur(source)
    await waitFor(() =>
      expect(update).toHaveBeenCalledExactlyOnceWith(7, 'haruka', {
        source: '春香 first',
        translation: 'Haruka',
        kind: 'person',
        enabled: true,
      }),
    )
    fireEvent.change(source, { target: { value: '春香 latest' } })
    fireEvent.blur(source)
    expect(update).toHaveBeenCalledTimes(1)

    await act(async () => first.resolve(glossary({ revision: 8 })))
    await waitFor(() =>
      expect(update).toHaveBeenLastCalledWith(8, 'haruka', {
        source: '春香 latest',
        translation: 'Haruka',
        kind: 'person',
        enabled: true,
      }),
    )
    await act(async () =>
      second.resolve(
        glossary({
          revision: 9,
          entries: [{ ...entries[0]!, source: '春香 latest' }, ...entries.slice(1)],
        }),
      ),
    )
    await waitFor(() =>
      expect(queryClient.getQueryData<GlossaryView>(glossaryKey)?.revision).toBe(9),
    )
  })

  it('updates the global and per-entry enable switches', async () => {
    const user = userEvent.setup()
    install()
    const globalToggle = vi
      .spyOn(commands, 'setGlossaryEnabled')
      .mockResolvedValue(glossary({ revision: 8, enabled: false }))
    const entryToggle = vi
      .spyOn(commands, 'updateGlossaryEntry')
      .mockResolvedValue(glossary({ revision: 9 }))
    render(<GlossaryPanel />)

    await user.click(screen.getByRole('switch', { name: 'Enable glossary' }))
    await waitFor(() => expect(globalToggle).toHaveBeenCalledWith(7, false))
    await user.click(screen.getByRole('switch', { name: 'Enable 春香' }))
    await waitFor(() =>
      expect(entryToggle).toHaveBeenCalledWith(8, 'haruka', {
        source: '春香',
        translation: 'Haruka',
        kind: 'person',
        enabled: false,
      }),
    )
  })

  it('filters by search, category, and not-present state', async () => {
    const user = userEvent.setup()
    install()
    render(<GlossaryPanel />)

    await user.type(screen.getByRole('searchbox', { name: 'Search glossary' }), 'Haruka')
    expect(screen.getByText('春香')).toBeInTheDocument()
    expect(screen.queryByText('月影学園')).not.toBeInTheDocument()
    await user.clear(screen.getByRole('searchbox', { name: 'Search glossary' }))

    await user.click(screen.getByRole('combobox', { name: 'Category filter' }))
    await user.click(screen.getByRole('option', { name: 'Organization' }))
    expect(screen.getByText('月影学園')).toBeInTheDocument()
    expect(screen.queryByText('春香')).not.toBeInTheDocument()

    await user.click(screen.getByRole('combobox', { name: 'State filter' }))
    await user.click(screen.getByRole('option', { name: 'Not present' }))
    expect(screen.getByText('月影学園')).toBeInTheDocument()
    expect(screen.getByText('Not present in the latest scan')).toBeInTheDocument()
  })

  it('adds a manual term and batch deletes selected entries', async () => {
    const user = userEvent.setup()
    install()
    const add = vi.spyOn(commands, 'addGlossaryEntry').mockResolvedValue(glossary({ revision: 8 }))
    const remove = vi
      .spyOn(commands, 'deleteGlossaryEntries')
      .mockResolvedValue(glossary({ revision: 9, entries: [entries[2]!] }))
    render(<GlossaryPanel />)

    await openActions()
    await user.click(screen.getByRole('menuitem', { name: 'Add term' }))
    const source = screen.getByRole('textbox', { name: 'New source term' })
    expect(source).toHaveFocus()
    await user.type(source, '新語')
    await user.type(screen.getByRole('textbox', { name: 'New translation' }), 'New term')
    await user.click(screen.getByRole('button', { name: 'Save term' }))
    await waitFor(() =>
      expect(add).toHaveBeenCalledWith(7, {
        source: '新語',
        translation: 'New term',
        kind: 'term',
        enabled: true,
      }),
    )

    await user.click(screen.getByRole('checkbox', { name: 'Select 春香' }))
    await user.click(screen.getByRole('checkbox', { name: 'Select 月影学園' }))
    await user.click(screen.getByRole('button', { name: 'Delete 2 selected' }))
    await waitFor(() => expect(remove).toHaveBeenCalledWith(8, ['haruka', 'academy']))
  })

  it('translates only eligible selected entries or all enabled untranslated entries', async () => {
    const user = userEvent.setup()
    install()
    const translate = vi
      .spyOn(commands, 'translateGlossaryEntries')
      .mockResolvedValue('translation-job')
    render(<GlossaryPanel />)

    await user.click(screen.getByRole('checkbox', { name: 'Select 春香' }))
    await user.click(screen.getByRole('checkbox', { name: 'Select 月影学園' }))
    await user.click(screen.getByRole('checkbox', { name: 'Select 星斬り' }))
    await user.click(screen.getByRole('button', { name: 'Translate selected' }))
    await waitFor(() => expect(translate).toHaveBeenLastCalledWith(7, ['academy']))

    await openActions()
    await user.click(screen.getByRole('menuitem', { name: 'Translate untranslated' }))
    await waitFor(() => expect(translate).toHaveBeenLastCalledWith(7, ['academy']))
  })

  it('previews import JSON, requires mismatch confirmation, and applies the chosen strategy', async () => {
    const user = userEvent.setup()
    install()
    const preview = vi.spyOn(commands, 'previewGlossaryImport').mockResolvedValue({
      added: 3,
      conflicting: 2,
      identical: 1,
      languageMismatches: [{ field: 'target', current: 'en', imported: 'zh-CN' }],
    })
    const apply = vi
      .spyOn(commands, 'applyGlossaryImport')
      .mockResolvedValue(glossary({ revision: 8 }))
    const view = render(<GlossaryPanel />)
    const input = view.container.querySelector('input[type="file"]') as HTMLInputElement

    const malformed = new File(['{bad'], 'bad.json', { type: 'application/json' })
    fireEvent.change(input, { target: { files: [malformed] } })
    expect(await screen.findByRole('alert')).toHaveTextContent('The glossary file is malformed')
    expect(preview).not.toHaveBeenCalled()

    const text = '{"format":"koharu-glossary","version":1,"entries":[]}'
    const file = new File([text], 'glossary.json', { type: 'application/json' })
    fireEvent.change(input, { target: { files: [file] } })
    await waitFor(() => expect(preview).toHaveBeenCalledWith(text))
    expect(await screen.findByText('3 to add')).toBeInTheDocument()
    expect(screen.getByText('2 conflicts')).toBeInTheDocument()
    expect(screen.getByText('1 identical')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Import glossary' })).toBeDisabled()

    await user.click(screen.getByRole('radio', { name: 'Replace existing' }))
    await user.click(screen.getByRole('checkbox', { name: 'Import despite language mismatch' }))
    await user.click(screen.getByRole('button', { name: 'Import glossary' }))
    await waitFor(() => expect(apply).toHaveBeenCalledWith(7, text, 'replace_existing', true))
  })

  it('exports through the generated shared download command', async () => {
    const user = userEvent.setup()
    install()
    const exportGlossary = vi.spyOn(commands, 'exportGlossary').mockResolvedValue(null)
    render(<GlossaryPanel />)

    await openActions()
    await user.click(screen.getByRole('menuitem', { name: 'Export glossary' }))

    await waitFor(() => expect(exportGlossary).toHaveBeenCalledOnce())
  })

  it('refreshes server state and reports a revision conflict without replaying the edit', async () => {
    install()
    const update = vi
      .spyOn(commands, 'updateGlossaryEntry')
      .mockRejectedValue(new Error('revision conflict'))
    const getGlossary = vi
      .spyOn(commands, 'getGlossary')
      .mockResolvedValue(glossary({ revision: 8 }))
    const errorToast = vi.spyOn(toast, 'add')
    render(<GlossaryPanel />)
    const source = screen.getByRole('textbox', { name: 'Source term 春香' })

    fireEvent.change(source, { target: { value: 'conflicting edit' } })
    fireEvent.blur(source)

    await waitFor(() =>
      expect(errorToast).toHaveBeenCalledWith(expect.objectContaining({ type: 'error' })),
    )
    await waitFor(() => expect(getGlossary).toHaveBeenCalled())
    expect(update).toHaveBeenCalledTimes(1)
  })
})
