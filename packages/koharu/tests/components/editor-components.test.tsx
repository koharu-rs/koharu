import { QueryClientProvider } from '@tanstack/react-query'
import { act, fireEvent, render as testingRender, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { StrictMode, type ReactNode } from 'react'
import { describe, expect, it, vi } from 'vitest'

import { TitleBar } from '@/components/app/TitleBar'
import { WindowControls } from '@/components/app/WindowChrome'
import { projectKey, queryClient } from '@/lib/queries'
import { receivePipeline } from '@/lib/store'
import { commands, type ComicInfo, type Preferences } from '@koharu/ipc/protocol'

const nativeWindow = vi.hoisted(() => ({
  close: vi.fn(async () => undefined),
  isMaximized: vi.fn(async () => false),
  minimize: vi.fn(async () => undefined),
  onResized: vi.fn(async () => () => undefined),
  startResizeDragging: vi.fn(async () => undefined),
  toggleMaximize: vi.fn(async () => undefined),
}))
const nativeOpenUrl = vi.hoisted(() => vi.fn(async () => undefined))
const nativeGetVersion = vi.hoisted(() => vi.fn(async () => '0.62.0'))

vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => nativeWindow }))
vi.mock('@tauri-apps/api/app', () => ({ getVersion: nativeGetVersion }))
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: nativeOpenUrl }))

function render(ui: ReactNode) {
  return testingRender(<QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>)
}

describe('window chrome', () => {
  it('registers one native resize listener in React strict mode', async () => {
    const unlisten = vi.fn()
    nativeWindow.onResized.mockClear()
    nativeWindow.onResized.mockResolvedValueOnce(unlisten)

    const view = render(
      <StrictMode>
        <WindowControls />
      </StrictMode>,
    )

    await waitFor(() => expect(nativeWindow.onResized).toHaveBeenCalledTimes(1))
    view.unmount()
    await waitFor(() => expect(unlisten).toHaveBeenCalledTimes(1))
  })

  it('starts native resize dragging from every frameless window edge', () => {
    nativeWindow.startResizeDragging.mockClear()
    const view = render(<WindowControls />)
    const directions = [
      'North',
      'South',
      'East',
      'West',
      'NorthEast',
      'NorthWest',
      'SouthEast',
      'SouthWest',
    ]

    for (const direction of directions) {
      fireEvent.pointerDown(
        view.container.querySelector(`[data-window-resize-handle="${direction}"]`)!,
        { button: 0 },
      )
    }

    expect(nativeWindow.startResizeDragging.mock.calls).toEqual(
      directions.map((direction) => [direction]),
    )
  })

  it('removes resize handles while the window is maximized', async () => {
    nativeWindow.isMaximized.mockResolvedValueOnce(true)
    const view = render(<WindowControls />)

    await waitFor(() =>
      expect(view.container.querySelector('[data-window-resize-handle]')).not.toBeInTheDocument(),
    )
  })

  it('opens community links through the Tauri opener plugin', async () => {
    nativeOpenUrl.mockClear()
    const user = userEvent.setup()
    render(<TitleBar />)

    await user.click(screen.getByRole('menuitem', { name: 'Help' }))
    fireEvent.click(await screen.findByRole('menuitem', { name: 'Discord' }))
    expect(nativeOpenUrl).toHaveBeenLastCalledWith('https://discord.gg/mHvHkxGnUY')

    await user.click(screen.getByRole('menuitem', { name: 'Help' }))
    fireEvent.click(await screen.findByRole('menuitem', { name: 'GitHub' }))
    expect(nativeOpenUrl).toHaveBeenLastCalledWith('https://github.com/koharu-rs/koharu')
  })

  it('shows the current version and author in About', async () => {
    const user = userEvent.setup()
    render(<TitleBar />)

    await user.click(screen.getByRole('menuitem', { name: 'Help' }))
    await user.click(await screen.findByRole('menuitem', { name: 'About' }))

    expect(await screen.findByRole('heading', { name: 'Koharu' })).toBeInTheDocument()
    expect(await screen.findByText('0.62.0')).toBeInTheDocument()
    expect(screen.getByText('Mayo Takanashi')).toBeInTheDocument()
    expect(nativeGetVersion).toHaveBeenCalledTimes(1)
  })

  it('opens unified export options pre-filled from preferences, hides lossy controls on PNG/lossless, and saves before exporting', async () => {
    const prefs: Preferences = {
      pipeline: {},
      renderer: { styles: {} },
      agent: {},
      providers: {},
      export: { cbz_codec: 'png', quality: 82, lossless: false },
    }
    const callOrder: string[] = []
    vi.spyOn(commands, 'getPreferences').mockResolvedValue(prefs)
    const saveSpy = vi.spyOn(commands, 'savePreferences').mockImplementation(async (next) => {
      callOrder.push('savePreferences')
      return next
    })
    const exportSpy = vi.spyOn(commands, 'export').mockImplementation(async () => {
      callOrder.push('export')
      return null
    })

    queryClient.setQueryData(projectKey, {
      name: 'Book',
      page: 'p1',
      pages: [{ id: 'p1', size: { width: 100, height: 100 }, texts: 0, translated: 0 }],
      history: { steps: [], current: 0 },
      revisions: {},
      meta: {},
    })

    const user = userEvent.setup()
    render(<TitleBar />)

    // Open CBZ export dialog
    await user.click(screen.getByRole('menuitem', { name: 'File' }))
    await user.click(await screen.findByRole('menuitem', { name: 'Export Project' }))
    fireEvent.click(await screen.findByRole('menuitem', { name: 'CBZ…' }))

    expect(await screen.findByRole('heading', { name: 'Export CBZ' })).toBeInTheDocument()
    const formatTrigger = await screen.findByRole('combobox', { name: 'Image format' })
    expect(exportSpy).not.toHaveBeenCalled()
    // PNG hides Lossless and Quality
    expect(screen.queryByRole('switch', { name: 'Lossless' })).not.toBeInTheDocument()
    expect(screen.queryByRole('group', { name: 'Quality' })).not.toBeInTheDocument()

    // Switching to WebP shows Lossless & Quality; toggling Lossless hides Quality
    await user.click(formatTrigger)
    await user.click(await screen.findByRole('option', { name: 'WebP' }))
    const losslessSwitch = await screen.findByRole('switch', { name: 'Lossless' })
    expect(screen.getByRole('group', { name: 'Quality' })).toBeInTheDocument()
    await user.click(losslessSwitch)
    expect(screen.queryByRole('group', { name: 'Quality' })).not.toBeInTheDocument()

    // Cancel discards changes
    await user.click(screen.getByRole('button', { name: 'Cancel' }))
    expect(saveSpy).not.toHaveBeenCalled()
    expect(exportSpy).not.toHaveBeenCalled()

    // Re-open, select JPEG XL, and confirm
    await user.click(screen.getByRole('menuitem', { name: 'File' }))
    await user.click(await screen.findByRole('menuitem', { name: 'Export Project' }))
    fireEvent.click(await screen.findByRole('menuitem', { name: 'CBZ…' }))
    await user.click(await screen.findByRole('combobox', { name: 'Image format' }))
    await user.click(await screen.findByRole('option', { name: 'JPEG XL' }))
    await user.click(screen.getByRole('button', { name: 'Export' }))

    await waitFor(() => expect(exportSpy).toHaveBeenCalledTimes(1))
    expect(saveSpy).toHaveBeenCalledWith({
      ...prefs,
      export: { cbz_codec: 'jxl', quality: 82, lossless: false },
    })
    expect(callOrder).toEqual(['savePreferences', 'export'])
    expect(exportSpy).toHaveBeenCalledWith(null, {
      cbz: { jxl: { quality: 82, lossless: false } },
    })
  })

  it('opens Metadata dialog from TitleBar, clears stale translation on original edit, expands More fields, and triggers Translate metadata', async () => {
    const prefs: Preferences = {
      pipeline: { translator: { target: 'en' } },
      renderer: { styles: {} },
      agent: {},
      providers: {},
      export: { cbz_codec: 'png', quality: 90, lossless: false },
    }
    const emptyInfo: ComicInfo = {
      title: { original: 'あつい夏', translated: 'Hot Summer' },
      series: { original: 'よつばと!', translated: null },
      number: '1',
      count: null,
      volume: 1,
      alternate_series: null,
      alternate_number: null,
      alternate_count: null,
      summary: null,
      notes: null,
      year: null,
      month: null,
      day: null,
      writer: 'あずまきよひこ',
      penciller: null,
      inker: null,
      colorist: null,
      letterer: null,
      cover_artist: null,
      editor: null,
      translator: null,
      publisher: null,
      imprint: null,
      genre: null,
      tags: null,
      web: null,
      page_count: 2,
      language_iso: 'ja',
      format: null,
      black_and_white: null,
      manga: 'YesAndRightToLeft',
      characters: null,
      teams: null,
      locations: null,
      scan_information: null,
      story_arc: null,
      story_arc_number: null,
      series_group: null,
      age_rating: null,
      pages: [],
      community_rating: null,
      main_character_or_team: null,
      review: null,
      gtin: null,
    }

    vi.spyOn(commands, 'getPreferences').mockResolvedValue(prefs)
    vi.spyOn(commands, 'getComicInfo').mockResolvedValue(emptyInfo)
    const saved: ComicInfo[] = []
    vi.spyOn(commands, 'setComicInfo').mockImplementation(async (next) => {
      saved.push(next)
      return {
        revision: 'r2',
        created: [],
        pages: { added: [], removed: [], changed: [], reordered: false },
        layers: [],
        history: { steps: [], current: 0 },
      }
    })
    const runPipelineSpy = vi.spyOn(commands, 'runPipeline').mockResolvedValue(42)

    const user = userEvent.setup()
    // Without an open project, File -> Metadata… is disabled and toolbar button is absent.
    const view = render(<TitleBar />)
    expect(screen.queryByRole('button', { name: 'Metadata' })).not.toBeInTheDocument()
    await user.click(screen.getByRole('menuitem', { name: 'File' }))
    expect(await screen.findByRole('menuitem', { name: 'Metadata…' })).toHaveAttribute(
      'aria-disabled',
      'true',
    )
    await user.keyboard('{Escape}')

    // With an open project, toolbar button opens MetadataPanel.
    queryClient.setQueryData(projectKey, {
      name: 'Book',
      page: 'p1',
      pages: [
        { id: 'p1', size: { width: 100, height: 100 }, texts: 0, translated: 0 },
        { id: 'p2', size: { width: 100, height: 100 }, texts: 0, translated: 0 },
      ],
      history: { steps: [], current: 0 },
      revisions: {},
      meta: {},
    })
    view.rerender(
      <QueryClientProvider client={queryClient}>
        <TitleBar />
      </QueryClientProvider>,
    )

    await user.click(screen.getByRole('button', { name: 'Metadata' }))
    expect(await screen.findByRole('heading', { name: 'Metadata' })).toBeInTheDocument()

    // Editing Title Original clears stale Title Translated and flushes before Translate metadata.
    const titleOriginal = await screen.findByLabelText('Title — Original')
    expect(titleOriginal).toHaveValue('あつい夏')
    expect(screen.getByLabelText('Title — Translated')).toHaveValue('Hot Summer')

    await user.clear(titleOriginal)
    await user.type(titleOriginal, 'あたらしい夏')
    fireEvent.blur(titleOriginal)

    await waitFor(() => expect(saved.length).toBeGreaterThanOrEqual(1))
    expect(saved[saved.length - 1].title).toEqual({
      original: 'あたらしい夏',
      translated: null,
    })

    // Expanding More fields reveals additional ComicInfo 2.1 fields and PageCount summary.
    expect(screen.queryByLabelText('Publisher')).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'More fields' }))
    expect(await screen.findByLabelText('Publisher')).toBeInTheDocument()
    expect(screen.getByText('2 pages — recomputed automatically on CBZ export')).toBeInTheDocument()

    // Clicking Translate metadata submits Target::Metadata + ['translate'] and shows active state.
    await user.click(screen.getByRole('button', { name: 'Translate metadata' }))
    await waitFor(() =>
      expect(runPipelineSpy).toHaveBeenCalledWith('metadata', ['translate'], 'normal'),
    )

    act(() => {
      receivePipeline({
        kind: 'queued',
        run: 42,
        steps: 1,
        priority: 'normal',
        scope: { kind: 'metadata' },
        stages: ['translate'],
        jobs: [{ page: null, stage: 'translate' }],
      })
    })
    expect(await screen.findByRole('button', { name: 'Translating…' })).toBeDisabled()

    act(() => {
      receivePipeline({ kind: 'waiting_for_download', run: 42, downloads: [1] })
    })
    expect(screen.getByRole('button', { name: 'Translating…' })).toBeDisabled()

    act(() => {
      receivePipeline({
        kind: 'started',
        run: 42,
        page: null,
        stage: 'translate',
        model: 'koharu/index-translate-9b',
      })
    })
    expect(screen.getByRole('button', { name: 'Translating…' })).toBeDisabled()
  })
})
