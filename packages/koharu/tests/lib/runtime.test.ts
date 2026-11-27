import { act, render, screen, waitFor } from '@testing-library/react'
import { createElement, StrictMode } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import Providers from '@/app/providers'
import { useProject } from '@/lib/queries'
import { useKoharuStore } from '@/lib/store'
import { commands, events, type Preferences } from '@koharu/ipc/protocol'

vi.mock('@tauri-apps/api/core', () => ({
  Channel: class {},
  invoke: vi.fn(),
}))

/** Each event's handlers, as the interface listened. */
const handlers = new Map<string, ((payload: unknown) => void)[]>()

/** Tells `payload` as the desktop would. */
function tell(name: keyof typeof events, payload: unknown): void {
  for (const handler of handlers.get(name) ?? []) handler(payload)
}

const preferences: Preferences = {
  pipeline: {},
  renderer: { styles: { antique: { Latn: { family: 'CCWildWords' } } } },
  agent: {},
  providers: {},
  export: { cbz_codec: 'png', quality: 90, lossless: false },
}

/** The desktop with `open` as its open project, empty. */
function desktop(open: () => string | null) {
  vi.spyOn(commands, 'getOpenProject').mockImplementation(async () => open())
  vi.spyOn(commands, 'getShownPage').mockResolvedValue(null)
  vi.spyOn(commands, 'query').mockImplementation(async (query) =>
    query.query === 'get_project'
      ? { id: 'project', version: '1', pages: [], meta: {} }
      : { steps: [], current: 0 },
  )
}

beforeEach(() => {
  handlers.clear()
  for (const name of Object.keys(events) as (keyof typeof events)[]) {
    vi.spyOn(events, name).mockImplementation(async (handler: (payload: never) => void) => {
      handlers.set(name, [...(handlers.get(name) ?? []), handler as (payload: unknown) => void])
      return () => undefined
    })
  }
  vi.spyOn(commands, 'listModelCatalog').mockResolvedValue({
    available_vram_bytes: null,
    total_vram_bytes: null,
    detect: [],
    recognize: [],
    inpaint: [],
    translate: [],
  })
  vi.spyOn(commands, 'getPreferences').mockResolvedValue(preferences)
  vi.spyOn(commands, 'getPipelineEvents').mockResolvedValue([])
  vi.spyOn(commands, 'listDownloads').mockResolvedValue([])
})

function ProjectProbe() {
  const project = useProject().data
  return createElement(
    'span',
    null,
    project === undefined ? 'Loading' : (project?.name ?? 'Closed'),
  )
}

describe('Tauri runtime', () => {
  it('reads the project that was open before the interface loaded', async () => {
    desktop(() => 'Book')
    const view = render(createElement(Providers, null, createElement(ProjectProbe)))

    expect(await screen.findByText('Book')).toBeInTheDocument()
    view.unmount()
  })

  it('listens once through Strict Mode effect replay', async () => {
    const runs = vi.spyOn(commands, 'getPipelineEvents').mockResolvedValue([])
    const view = render(
      createElement(StrictMode, null, createElement(Providers, null, createElement('div'))),
    )

    await waitFor(() => expect(runs).toHaveBeenCalledTimes(1))
    expect(handlers.get('pipeline')).toHaveLength(1)
    act(() => {
      tell('pipeline', {
        kind: 'queued',
        run: 1,
        steps: 4,
        priority: 'normal',
        scope: { kind: 'project' },
        stages: ['detect'],
        jobs: [{ page: 'page', stage: 'detect' }],
      })
      tell('pipeline', {
        kind: 'started',
        run: 1,
        page: 'page',
        stage: 'detect',
        model: 'koharu-layout-highres',
      })
    })

    expect(useKoharuStore.getState().jobs[1]).toMatchObject({
      state: 'running',
      total: 4,
      stage: 'detect',
    })
    view.unmount()
  })

  it('applies the canvas and download events directly to the store', async () => {
    useKoharuStore.setState({ downloads: {} })
    const view = render(createElement(Providers, null, createElement('div')))
    await waitFor(() => expect(commands.getPipelineEvents).toHaveBeenCalledTimes(1))

    act(() => {
      tell('download', {
        id: 7,
        model_id: 'model.bin',
        model_name: 'model.bin',
        stage: null,
        completed: 25,
        total: 100,
        bytes_per_sec: 512,
        files_done: 0,
        files_total: 1,
        state: 'Running',
      })
      tell('camera', { zoom: 0.5, x: 12, y: 34 })
      tell('selection', ['layer'])
    })
    expect(useKoharuStore.getState().downloads[7]).toMatchObject({
      model_name: 'model.bin',
      completed: 25,
      total: 100,
    })
    expect(useKoharuStore.getState().camera).toEqual({ zoom: 0.5, x: 12, y: 34 })
    expect(useKoharuStore.getState().selectedLayers).toEqual(['layer'])
    view.unmount()
  })

  it('hydrates existing downloads on mount via listDownloads', async () => {
    vi.spyOn(commands, 'listDownloads').mockResolvedValue([
      {
        id: 3,
        model_id: 'lama',
        model_name: 'LaMa',
        stage: 'inpaint',
        completed: 1000,
        total: 1000,
        bytes_per_sec: 0,
        files_done: 1,
        files_total: 1,
        state: 'Finished',
      },
      {
        id: 4,
        model_id: 'hayai',
        model_name: 'Hayai OCR',
        stage: 'recognize',
        completed: 250,
        total: 1000,
        bytes_per_sec: 500,
        files_done: 0,
        files_total: 2,
        state: 'Running',
      },
    ])
    const view = render(createElement(Providers, null, createElement('div')))
    await waitFor(() => expect(useKoharuStore.getState().downloads[3]).toBeDefined())
    expect(useKoharuStore.getState().downloads[3]).toMatchObject({ state: 'Finished' })
    expect(useKoharuStore.getState().downloads[4]).toMatchObject({
      state: 'Running',
      bytes_per_sec: 500,
    })
    view.unmount()
  })

  it('shows a project the desktop tells it opened', async () => {
    let open: string | null = null
    desktop(() => open)
    const view = render(createElement(Providers, null, createElement(ProjectProbe)))
    expect(await screen.findByText('Closed')).toBeInTheDocument()

    open = 'Book'
    act(() => tell('project', 'Book'))

    expect(await screen.findByText('Book')).toBeInTheDocument()
    view.unmount()
  })

  it('tracks fileDrag over PageRail midpoints, empty rail, workspace fallback, and leave cleanup', async () => {
    desktop(() => 'Book')
    const importSpy = vi
      .spyOn(commands, 'importPaths')
      .mockResolvedValue({ added_pages: 1, skipped_files: [] })

    const rail = document.createElement('aside')
    rail.setAttribute('data-page-rail', '')
    rail.getBoundingClientRect = () => new DOMRect(0, 40, 160, 400)
    const body = document.createElement('div')
    body.setAttribute('data-page-rail-body', '')
    body.getBoundingClientRect = () => new DOMRect(0, 80, 160, 360)
    rail.appendChild(body)
    document.body.appendChild(rail)

    const view = render(createElement(Providers, null, createElement(ProjectProbe)))
    expect(await screen.findByText('Book')).toBeInTheDocument()

    // Empty rail targets slot 0.
    act(() => tell('fileDrag', { phase: 'over', x: 50, y: 120 }))
    expect(useKoharuStore.getState().fileDrag).toMatchObject({
      target: 'rail',
      insertionIndex: 0,
    })

    // Populated rail resolves exact midpoint insertion index.
    const row0 = document.createElement('article')
    row0.setAttribute('data-page-index', '0')
    row0.getBoundingClientRect = () => new DOMRect(0, 80, 160, 60)
    const row1 = document.createElement('article')
    row1.setAttribute('data-page-index', '1')
    row1.getBoundingClientRect = () => new DOMRect(0, 142, 160, 60)
    body.append(row0, row1)

    act(() => tell('fileDrag', { phase: 'over', x: 50, y: 160 }))
    expect(useKoharuStore.getState().fileDrag).toMatchObject({
      target: 'rail',
      insertionIndex: 1,
    })

    // Leave clears indicator without importing.
    act(() => tell('fileDrag', { phase: 'leave' }))
    expect(useKoharuStore.getState().fileDrag).toBeNull()
    expect(importSpy).not.toHaveBeenCalled()

    // Workspace drop appends (`insertAt = null`), rail drop inserts at resolved slot (`2`).
    act(() => tell('fileDrag', { phase: 'over', x: 300, y: 200 }))
    expect(useKoharuStore.getState().fileDrag).toMatchObject({
      target: 'workspace',
      insertionIndex: null,
    })
    act(() => tell('fileDrag', { phase: 'drop', x: 300, y: 200, paths: ['/tmp/a.png'] }))
    await waitFor(() => expect(importSpy).toHaveBeenCalledWith(['/tmp/a.png'], null))

    act(() => tell('fileDrag', { phase: 'drop', x: 50, y: 190, paths: ['/tmp/b.png'] }))
    await waitFor(() => expect(importSpy).toHaveBeenCalledWith(['/tmp/b.png'], 2))

    rail.remove()
    view.unmount()
  })
})
