import { act, render, screen, waitFor } from '@testing-library/react'
import { createElement, StrictMode } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import Providers from '@/app/providers'
import { call } from '@/lib/backend'
import { useProject } from '@/lib/queries'
import { useKoharuStore } from '@/lib/store'
import {
  commands,
  type Preferences,
  type ProjectInfo,
  type StartupState,
} from '@koharu/bridge/protocol'

vi.mock('@tauri-apps/api/core', () => ({
  Channel: class<T> {
    onmessage: (payload: T) => void

    constructor(handler: (payload: T) => void) {
      this.onmessage = handler
    }
  },
  invoke: vi.fn(),
}))

const preferences: Preferences = {
  pipeline: {},
  renderer: { font_families: ['CCWildWords'] },
  agent: {},
  providers: {},
}

const project: ProjectInfo = {
  name: 'Book',
  revision: '3',
  page: null,
  pages: [],
  history: { steps: [], current: 0 },
}

beforeEach(() => {
  vi.spyOn(commands, 'getModels').mockResolvedValue([])
  vi.spyOn(commands, 'getPreferences').mockResolvedValue(preferences)
})

const startupState = (open: ProjectInfo | null = null): StartupState => ({
  project: open,
  jobs: [],
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
  it('takes the open project from the startup state', async () => {
    const pending = deferred<StartupState>()
    vi.spyOn(commands, 'getProject').mockReturnValue(new Promise(() => undefined))
    vi.spyOn(commands, 'getPage').mockResolvedValue(null)
    vi.spyOn(commands, 'subscribe').mockReturnValue(pending.promise)
    const view = render(createElement(Providers, null, createElement(ProjectProbe)))

    expect(screen.queryByText('Loading')).toBeNull()
    pending.resolve(startupState(project))
    expect(await screen.findByText('Book')).toBeInTheDocument()
    view.unmount()
  })

  it('keeps one live job channel through Strict Mode effect replay', async () => {
    const binding = vi.spyOn(commands, 'subscribe').mockResolvedValue(startupState())
    const view = render(
      createElement(StrictMode, null, createElement(Providers, null, createElement('div'))),
    )

    await waitFor(() => expect(binding).toHaveBeenCalledTimes(1))
    const [, , jobChannel] = binding.mock.calls[0]!
    act(() => {
      jobChannel.onmessage({
        id: 1,
        state: 'running',
        completed: 0,
        total: 4,
        page: 'page',
        stage: 'detect',
        error: null,
      })
    })

    expect(useKoharuStore.getState().jobs[1]).toMatchObject({ state: 'running', total: 4 })
    view.unmount()
  })

  it('passes the managed project name to open', async () => {
    const open = vi.spyOn(commands, 'openProject').mockResolvedValue(null)
    await expect(call(commands.openProject, 'Volume 1')).resolves.toBeNull()
    expect(open).toHaveBeenCalledWith('Volume 1')
  })

  it('applies the canvas and download channels directly to the store', async () => {
    useKoharuStore.setState({ downloads: {} })
    const binding = vi.spyOn(commands, 'subscribe').mockResolvedValue(startupState())
    const view = render(createElement(Providers, null, createElement('div')))
    await waitFor(() => expect(binding).toHaveBeenCalledTimes(1))
    const [, canvasChannel, , downloadChannel] = binding.mock.calls[0]!

    act(() => {
      downloadChannel.onmessage({
        id: 7,
        state: 'Running',
        name: 'model.bin',
        completed: 25,
        total: 100,
      })
      canvasChannel.onmessage({ type: 'camera', zoom: 0.5, x: 12, y: 34 })
      canvasChannel.onmessage({ type: 'selection', layers: ['layer'] })
    })
    expect(useKoharuStore.getState().downloads[7]).toMatchObject({ completed: 25, total: 100 })
    expect(useKoharuStore.getState().camera).toEqual({ zoom: 0.5, x: 12, y: 34 })
    expect(useKoharuStore.getState().selectedLayers).toEqual(['layer'])
    view.unmount()
  })

  it('shows a project the desktop publishes after a change', async () => {
    vi.spyOn(commands, 'getPage').mockResolvedValue(null)
    const binding = vi.spyOn(commands, 'subscribe').mockResolvedValue(startupState())
    const view = render(createElement(Providers, null, createElement(ProjectProbe)))
    expect(await screen.findByText('Closed')).toBeInTheDocument()

    const [projectChannel] = binding.mock.calls[0]!
    act(() => projectChannel.onmessage(project))

    expect(await screen.findByText('Book')).toBeInTheDocument()
    view.unmount()
  })
})

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}
