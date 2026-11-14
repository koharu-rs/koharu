import { act, render, screen, waitFor } from '@testing-library/react'
import { createElement, StrictMode } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import Providers from '@/app/providers'
import { call } from '@/lib/backend'
import { useProject } from '@/lib/queries'
import { useKoharuStore } from '@/lib/store'
import { commands, events, type Preferences } from '@koharu/bridge/protocol'

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
  renderer: { font_families: ['CCWildWords'] },
  agent: {},
  providers: {},
}

/** The desktop with `open` as its open project, empty. */
function desktop(open: () => string | null) {
  vi.spyOn(commands, 'getOpenProject').mockImplementation(async () => open())
  vi.spyOn(commands, 'getShownPage').mockResolvedValue(null)
  vi.spyOn(commands, 'query').mockImplementation(async (query) =>
    query.query === 'get_project'
      ? { id: 'project', version: '1', pages: [] }
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
  vi.spyOn(commands, 'getModels').mockResolvedValue([])
  vi.spyOn(commands, 'getPreferences').mockResolvedValue(preferences)
  vi.spyOn(commands, 'getRuns').mockResolvedValue([])
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
    const runs = vi.spyOn(commands, 'getRuns').mockResolvedValue([])
    const view = render(
      createElement(StrictMode, null, createElement(Providers, null, createElement('div'))),
    )

    await waitFor(() => expect(runs).toHaveBeenCalledTimes(1))
    expect(handlers.get('pipeline')).toHaveLength(1)
    act(() => {
      tell('pipeline', { kind: 'queued', run: 1, steps: 4 })
      tell('pipeline', { kind: 'started', run: 1, page: 'page', stage: 'detect' })
    })

    expect(useKoharuStore.getState().jobs[1]).toMatchObject({
      state: 'running',
      total: 4,
      stage: 'detect',
    })
    view.unmount()
  })

  it('passes the managed project name to open', async () => {
    const open = vi.spyOn(commands, 'openProject').mockResolvedValue(null)
    await expect(call(commands.openProject, 'Volume 1')).resolves.toBeNull()
    expect(open).toHaveBeenCalledWith('Volume 1')
  })

  it('applies the canvas and download events directly to the store', async () => {
    useKoharuStore.setState({ downloads: {} })
    const view = render(createElement(Providers, null, createElement('div')))
    await waitFor(() => expect(commands.getRuns).toHaveBeenCalledTimes(1))

    act(() => {
      tell('download', { id: 7, state: 'Running', name: 'model.bin', completed: 25, total: 100 })
      tell('camera', { zoom: 0.5, x: 12, y: 34 })
      tell('selection', ['layer'])
    })
    expect(useKoharuStore.getState().downloads[7]).toMatchObject({ completed: 25, total: 100 })
    expect(useKoharuStore.getState().camera).toEqual({ zoom: 0.5, x: 12, y: 34 })
    expect(useKoharuStore.getState().selectedLayers).toEqual(['layer'])
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
})
