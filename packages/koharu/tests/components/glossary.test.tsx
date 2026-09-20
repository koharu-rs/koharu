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

const otherProject: ProjectInfo = {
  ...project,
  name: 'Other Book',
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

function glossaryWithSource(source: string, revision = 7): GlossaryView {
  return glossary({
    revision,
    entries: entries.map((entry) => ({
      ...entry,
      revision,
      source: entry.id === 'haruka' ? source : entry.source,
    })),
  })
}

function install(data: GlossaryView = glossary()) {
  queryClient.setQueryData(projectKey, project)
  queryClient.setQueryData(glossaryKey(project.name), data)
  vi.spyOn(commands, 'getProject').mockResolvedValue(project)
  vi.spyOn(commands, 'getGlossary').mockImplementation(
    async () => queryClient.getQueryData<GlossaryView>(glossaryKey(project.name)) ?? data,
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

  it('never renders the previous project glossary while the next project loads', async () => {
    install(glossaryWithSource('Book term'))
    const nextGlossary = Promise.withResolvers<GlossaryView>()
    vi.spyOn(commands, 'getGlossary').mockReturnValue(nextGlossary.promise)
    render(<GlossaryPanel />)
    expect(screen.getByText('Book term')).toBeInTheDocument()

    act(() => queryClient.setQueryData(projectKey, otherProject))

    expect(await screen.findByText('Loading…')).toBeInTheDocument()
    expect(screen.queryByText('Book term')).not.toBeInTheDocument()
    nextGlossary.resolve(glossaryWithSource('Other term'))
    expect(await screen.findByText('Other term')).toBeInTheDocument()
    expect(queryClient.getQueryData(glossaryKey(project.name))).toMatchObject({ revision: 7 })
    expect(queryClient.getQueryData(glossaryKey(otherProject.name))).toMatchObject({ revision: 7 })
  })

  it('ignores an old project update after a coincident new-project revision refresh', async () => {
    install(glossaryWithSource('Book term'))
    const oldUpdate = Promise.withResolvers<GlossaryView>()
    const newProjectRefresh = Promise.withResolvers<GlossaryView>()
    vi.spyOn(commands, 'updateGlossaryEntry').mockReturnValue(oldUpdate.promise)
    const getProject = vi.spyOn(commands, 'getProject').mockResolvedValue(otherProject)
    vi.spyOn(commands, 'getGlossary').mockReturnValue(newProjectRefresh.promise)
    render(<GlossaryPanel />)

    const source = screen.getByRole('textbox', { name: 'Source term Book term' })
    fireEvent.change(source, { target: { value: 'Old pending edit' } })
    fireEvent.blur(source)
    await waitFor(() => expect(commands.updateGlossaryEntry).toHaveBeenCalledOnce())

    const otherView = glossaryWithSource('Other term', 8)
    act(() => {
      queryClient.setQueryData(glossaryKey(otherProject.name), otherView)
      queryClient.setQueryData(projectKey, { ...otherProject, revision: 8 })
    })
    act(() => {
      void queryClient.invalidateQueries({ queryKey: glossaryKey(otherProject.name), exact: true })
    })
    expect(
      await screen.findByRole('textbox', { name: 'Source term Other term' }),
    ).toBeInTheDocument()
    expect(screen.getByRole('switch', { name: 'Enable glossary' })).toHaveAttribute(
      'aria-disabled',
      'true',
    )

    await act(async () => oldUpdate.resolve(glossaryWithSource('Old pending edit', 8)))

    expect(getProject).not.toHaveBeenCalled()
    expect(queryClient.getQueryData(glossaryKey(otherProject.name))).toEqual(otherView)
    expect(screen.getByText('Other term')).toBeInTheDocument()
    expect(screen.queryByText('Old pending edit')).not.toBeInTheDocument()

    newProjectRefresh.resolve(glossaryWithSource('Other refreshed term', 8))
    expect(await screen.findByText('Other refreshed term')).toBeInTheDocument()
  })

  it('cancels a queued update owned by the previous project', async () => {
    install(glossaryWithSource('Book term'))
    vi.spyOn(commands, 'getProject').mockImplementation(
      async () => queryClient.getQueryData<ProjectInfo | null>(projectKey) ?? null,
    )
    vi.spyOn(commands, 'getGlossary').mockImplementation(async () => {
      const current = queryClient.getQueryData<ProjectInfo | null>(projectKey)
      return queryClient.getQueryData<GlossaryView>(glossaryKey(current?.name))!
    })
    const firstUpdate = Promise.withResolvers<GlossaryView>()
    const otherUpdated = glossaryWithSource('Other updated term', 8)
    const update = vi
      .spyOn(commands, 'updateGlossaryEntry')
      .mockReturnValueOnce(firstUpdate.promise)
      .mockResolvedValueOnce(otherUpdated)
    const errorToast = vi.spyOn(toast, 'add')
    const unhandled = vi.fn()
    window.addEventListener('unhandledrejection', unhandled)
    render(<GlossaryPanel />)

    const source = screen.getByRole('textbox', { name: 'Source term Book term' })
    const translation = screen.getByRole('textbox', { name: 'Translation for Book term' })
    fireEvent.change(source, { target: { value: 'Book first edit' } })
    fireEvent.blur(source)
    await waitFor(() => expect(update).toHaveBeenCalledTimes(1))
    fireEvent.change(translation, { target: { value: 'Book queued edit' } })
    fireEvent.blur(translation)
    expect(update).toHaveBeenCalledTimes(1)

    const otherView = glossaryWithSource('Other term')
    act(() => {
      queryClient.setQueryData(glossaryKey(otherProject.name), otherView)
      queryClient.setQueryData(projectKey, otherProject)
    })
    expect(
      await screen.findByRole('textbox', { name: 'Source term Other term' }),
    ).toBeInTheDocument()

    await act(async () => firstUpdate.resolve(glossaryWithSource('Book first edit', 8)))
    await act(async () => undefined)

    expect(update).toHaveBeenCalledTimes(1)
    expect(queryClient.getQueryData(glossaryKey(otherProject.name))).toEqual(otherView)
    expect(screen.getByText('Other term')).toBeInTheDocument()
    expect(screen.queryByText('Book first edit')).not.toBeInTheDocument()
    expect(errorToast).not.toHaveBeenCalled()
    expect(unhandled).not.toHaveBeenCalled()

    const otherSource = screen.getByRole('textbox', { name: 'Source term Other term' })
    fireEvent.change(otherSource, { target: { value: 'Other updated term' } })
    fireEvent.blur(otherSource)
    await waitFor(() =>
      expect(update).toHaveBeenNthCalledWith(2, 7, 'haruka', {
        source: 'Other updated term',
        translation: 'Haruka',
        kind: 'person',
        enabled: true,
      }),
    )
    await waitFor(() =>
      expect(queryClient.getQueryData(glossaryKey(otherProject.name))).toEqual(otherUpdated),
    )
    expect(screen.getByRole('textbox', { name: 'Source term Other updated term' })).toHaveValue(
      'Other updated term',
    )
    expect(errorToast).not.toHaveBeenCalled()
    expect(unhandled).not.toHaveBeenCalled()
    window.removeEventListener('unhandledrejection', unhandled)
  })

  it('offers a first scan and changes to rescan when the glossary is stale', async () => {
    const user = userEvent.setup()
    install(glossary({ savedSourceFingerprint: null, entries: [] }))
    const scan = vi.spyOn(commands, 'scanGlossary').mockResolvedValue('scan-job')
    const view = render(<GlossaryPanel />)

    expect(screen.getByText('Not scanned')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Scan glossary' }))
    await waitFor(() => expect(scan).toHaveBeenCalledOnce())

    queryClient.setQueryData(glossaryKey(project.name), glossary({ stale: true }))
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
      expect(queryClient.getQueryData<GlossaryView>(glossaryKey(project.name))?.revision).toBe(9),
    )
  })

  it('does not publish an older full view over a queued edit to another entry', async () => {
    install()
    const first = Promise.withResolvers<GlossaryView>()
    const second = Promise.withResolvers<GlossaryView>()
    const update = vi
      .spyOn(commands, 'updateGlossaryEntry')
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise)
    render(<GlossaryPanel />)
    const source = screen.getByRole('textbox', { name: 'Source term 春香' })
    const translation = screen.getByRole('textbox', { name: 'Translation for 月影学園' })

    fireEvent.change(source, { target: { value: '春香 changed' } })
    fireEvent.blur(source)
    await waitFor(() => expect(update).toHaveBeenCalledTimes(1))
    fireEvent.change(translation, { target: { value: 'Moonlight Academy' } })
    fireEvent.blur(translation)
    expect(update).toHaveBeenCalledTimes(1)

    const firstView = glossary({
      revision: 8,
      entries: entries.map((entry) =>
        entry.id === 'haruka' ? { ...entry, revision: 8, source: '春香 changed' } : entry,
      ),
    })
    await act(async () => first.resolve(firstView))

    expect(translation).toHaveValue('Moonlight Academy')
    await waitFor(() =>
      expect(update).toHaveBeenNthCalledWith(2, 8, 'academy', {
        source: '月影学園',
        translation: 'Moonlight Academy',
        kind: 'organization',
        enabled: true,
      }),
    )
    const secondView = glossary({
      revision: 9,
      entries: firstView.entries.map((entry) =>
        entry.id === 'academy'
          ? { ...entry, revision: 9, translation: 'Moonlight Academy' }
          : { ...entry, revision: 9 },
      ),
    })
    await act(async () => second.resolve(secondView))
    expect(queryClient.getQueryData(glossaryKey(project.name))).toEqual(secondView)
  })

  it('cancels source and translation edits with Escape without mutating', async () => {
    install()
    const update = vi.spyOn(commands, 'updateGlossaryEntry').mockResolvedValue(glossary())
    render(<GlossaryPanel />)
    const source = screen.getByRole('textbox', { name: 'Source term 春香' })
    const translation = screen.getByRole('textbox', { name: 'Translation for 春香' })

    fireEvent.focus(source)
    fireEvent.change(source, { target: { value: 'cancel source' } })
    fireEvent.keyDown(source, { key: 'Escape' })
    expect(source).toHaveValue('春香')
    fireEvent.focus(translation)
    fireEvent.change(translation, { target: { value: 'cancel translation' } })
    fireEvent.keyDown(translation, { key: 'Escape' })
    expect(translation).toHaveValue('Haruka')
    await act(async () => undefined)
    expect(update).not.toHaveBeenCalled()
  })

  it('saves source with Enter and translation with normal blur', async () => {
    const user = userEvent.setup()
    install()
    const update = vi
      .spyOn(commands, 'updateGlossaryEntry')
      .mockResolvedValueOnce(glossaryWithSource('Entered source', 8))
      .mockResolvedValueOnce(
        glossary({
          revision: 9,
          entries: entries.map((entry) =>
            entry.id === 'haruka'
              ? {
                  ...entry,
                  revision: 9,
                  source: 'Entered source',
                  translation: 'Blurred translation',
                }
              : { ...entry, revision: 9 },
          ),
        }),
      )
    render(<GlossaryPanel />)
    const source = screen.getByRole('textbox', { name: 'Source term 春香' })

    await user.clear(source)
    await user.type(source, 'Entered source{Enter}')
    await waitFor(() =>
      expect(update).toHaveBeenNthCalledWith(1, 7, 'haruka', {
        source: 'Entered source',
        translation: 'Haruka',
        kind: 'person',
        enabled: true,
      }),
    )

    const translation = await screen.findByRole('textbox', {
      name: 'Translation for Entered source',
    })
    await user.clear(translation)
    await user.type(translation, 'Blurred translation')
    await user.tab()
    await waitFor(() =>
      expect(update).toHaveBeenNthCalledWith(2, 8, 'haruka', {
        source: 'Entered source',
        translation: 'Blurred translation',
        kind: 'person',
        enabled: true,
      }),
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
    await user.click(await screen.findByRole('option', { name: 'Organization' }))
    expect(screen.getByText('月影学園')).toBeInTheDocument()
    expect(screen.queryByText('春香')).not.toBeInTheDocument()

    await user.click(screen.getByRole('combobox', { name: 'State filter' }))
    await user.click(await screen.findByRole('option', { name: 'Not present' }))
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

  it('keeps only the latest overlapping import preview and disables conflicting actions', async () => {
    install()
    const first = Promise.withResolvers<{
      added: number
      conflicting: number
      identical: number
      languageMismatches: []
    }>()
    const second = Promise.withResolvers<{
      added: number
      conflicting: number
      identical: number
      languageMismatches: []
    }>()
    const preview = vi
      .spyOn(commands, 'previewGlossaryImport')
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise)
    const apply = vi
      .spyOn(commands, 'applyGlossaryImport')
      .mockResolvedValue(glossary({ revision: 8 }))
    const view = render(<GlossaryPanel />)
    const input = view.container.querySelector('input[type="file"]') as HTMLInputElement
    const firstDocument = '{"format":"koharu-glossary","version":1,"entries":[{"source":"first"}]}'
    const secondDocument =
      '{"format":"koharu-glossary","version":1,"entries":[{"source":"second"}]}'

    fireEvent.change(input, {
      target: { files: [new File([firstDocument], 'first.json', { type: 'application/json' })] },
    })
    await waitFor(() => expect(preview).toHaveBeenCalledWith(firstDocument))
    expect(screen.getByRole('button', { name: 'Glossary actions' })).toBeDisabled()
    fireEvent.change(input, {
      target: { files: [new File([secondDocument], 'second.json', { type: 'application/json' })] },
    })
    await waitFor(() => expect(preview).toHaveBeenCalledWith(secondDocument))

    await act(async () =>
      second.resolve({ added: 9, conflicting: 2, identical: 3, languageMismatches: [] }),
    )
    expect(await screen.findByText('9 to add')).toBeInTheDocument()
    await act(async () =>
      first.resolve({ added: 1, conflicting: 0, identical: 0, languageMismatches: [] }),
    )
    expect(screen.getByText('9 to add')).toBeInTheDocument()
    expect(screen.queryByText('1 to add')).not.toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: 'Import glossary' }))
    await waitFor(() =>
      expect(apply).toHaveBeenCalledWith(7, secondDocument, 'keep_existing', false),
    )
  })

  it('ignores an obsolete import preview rejection after switching projects', async () => {
    install()
    const pending = Promise.withResolvers<never>()
    vi.spyOn(commands, 'previewGlossaryImport').mockReturnValue(pending.promise)
    const errorToast = vi.spyOn(toast, 'add')
    const view = render(<GlossaryPanel />)
    const input = view.container.querySelector('input[type="file"]') as HTMLInputElement
    const document = '{"format":"koharu-glossary","version":1,"entries":[]}'

    fireEvent.change(input, {
      target: { files: [new File([document], 'old.json', { type: 'application/json' })] },
    })
    await waitFor(() => expect(commands.previewGlossaryImport).toHaveBeenCalledOnce())
    act(() => {
      queryClient.setQueryData(glossaryKey(otherProject.name), glossaryWithSource('Other term'))
      queryClient.setQueryData(projectKey, otherProject)
    })
    expect(await screen.findByText('Other term')).toBeInTheDocument()
    await act(async () => pending.reject(new Error('old project preview failed')))

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    expect(errorToast).not.toHaveBeenCalled()
    expect(screen.getByText('Other term')).toBeInTheDocument()
  })

  it('reports a valid JSON document rejected by backend schema validation', async () => {
    install()
    vi.spyOn(commands, 'previewGlossaryImport').mockRejectedValue(new Error('invalid schema'))
    const errorToast = vi.spyOn(toast, 'add')
    const view = render(<GlossaryPanel />)
    const input = view.container.querySelector('input[type="file"]') as HTMLInputElement

    fireEvent.change(input, {
      target: { files: [new File(['{}'], 'schema.json', { type: 'application/json' })] },
    })

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'The glossary could not be imported.',
    )
    expect(errorToast).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'error', description: 'invalid schema' }),
    )
  })

  it('keeps the import dialog open and handles an apply rejection', async () => {
    install()
    vi.spyOn(commands, 'previewGlossaryImport').mockResolvedValue({
      added: 3,
      conflicting: 0,
      identical: 0,
      languageMismatches: [],
    })
    const apply = vi
      .spyOn(commands, 'applyGlossaryImport')
      .mockRejectedValue(new Error('revision conflict'))
    const errorToast = vi.spyOn(toast, 'add')
    const unhandled = vi.fn()
    window.addEventListener('unhandledrejection', unhandled)
    const view = render(<GlossaryPanel />)
    const input = view.container.querySelector('input[type="file"]') as HTMLInputElement
    const document = '{"format":"koharu-glossary","version":1,"entries":[]}'

    fireEvent.change(input, {
      target: { files: [new File([document], 'glossary.json', { type: 'application/json' })] },
    })
    expect(await screen.findByText('3 to add')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Import glossary' }))
    await waitFor(() => expect(apply).toHaveBeenCalledOnce())
    await waitFor(() =>
      expect(errorToast).toHaveBeenCalledWith(
        expect.objectContaining({ type: 'error', description: 'revision conflict' }),
      ),
    )

    expect(screen.getByRole('dialog')).toBeInTheDocument()
    expect(screen.getByText('3 to add')).toBeInTheDocument()
    expect(unhandled).not.toHaveBeenCalled()
    window.removeEventListener('unhandledrejection', unhandled)
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
