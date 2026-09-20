import { act, render, screen, waitFor } from '@testing-library/react'
import { createElement, StrictMode } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import Providers from '@/app/providers'
import { call } from '@/lib/backend'
import { glossaryKey, queryClient, useGlossary, useProject } from '@/lib/queries'
import { useKoharuStore } from '@/lib/store'
import { commands } from '@koharu/bridge'
import type {
  GlossaryView,
  Job,
  Preferences,
  ProjectInfo,
  StartupState,
} from '@koharu/bridge/protocol'

const preferences: Preferences = {
  pipeline: {
    detection: { model: 'koharu-layout-rfdetr-seg-2xl' },
    ocr: { model: 'paddleocr-vl-1.6' },
    translation: {
      model: {
        provider: 'local',
        model: 'lfm2.5-1.2b-instruct',
        quantization: null,
        vision: false,
        reasoning: false,
      },
      generation: { vision: true, reasoning: true },
      target_language: 'en-US',
      instructions: null,
    },
    inpainting: { model: 'lama' },
    processor: {},
  },
  providers: {
    entries: [],
  },
  typesetting: {
    font_families: ['CCWildWords', 'Adobe 黑体 Std'],
  },
  languages: [],
}

const project: ProjectInfo = {
  name: 'Book',
  revision: 3,
  active_page: null,
  can_undo: true,
  can_redo: false,
}

const glossary: GlossaryView = {
  revision: 3,
  enabled: true,
  stale: false,
  sourceLanguage: 'ja',
  targetLanguage: 'en',
  savedSourceFingerprint: 'saved',
  currentSourceFingerprint: 'saved',
  entries: [],
}

beforeEach(() => {
  vi.spyOn(commands, 'getTranslationModels').mockResolvedValue([])
})

const startupState = (): StartupState => ({
  native_dialogs: false,
  preferences,
  jobs: [],
  canvas: {
    page: null,
    revision: null,
    generation: 0,
    size: [0, 0],
    element_frames: [],
  },
})

async function start() {
  const pending = deferred<StartupState>()
  const binding = vi.spyOn(commands, 'subscribe').mockReturnValue(pending.promise)
  const view = render(createElement(Providers, null, createElement('div')))
  pending.resolve(startupState())
  await waitFor(() => expect(useKoharuStore.getState().preferences).toBe(preferences))
  return { binding, dispose: view.unmount }
}

function ProjectProbe() {
  const project = useProject().data
  return createElement(
    'span',
    null,
    project === undefined ? 'Loading' : (project?.name ?? 'Closed'),
  )
}

function GlossaryProbe() {
  const project = useProject().data
  const glossary = useGlossary(project?.name).data
  return createElement('span', null, glossary ? `Glossary ${glossary.revision}` : 'No glossary')
}

describe('application runtime', () => {
  it('keeps the project unresolved until its backend query returns', async () => {
    const projectPending = deferred<ProjectInfo | null>()
    vi.spyOn(commands, 'getProject').mockReturnValue(projectPending.promise)
    vi.spyOn(commands, 'subscribe').mockResolvedValue(startupState())
    const view = render(createElement(Providers, null, createElement(ProjectProbe)))

    expect(await screen.findByText('Loading')).toBeInTheDocument()
    projectPending.resolve(null)
    expect(await screen.findByText('Closed')).toBeInTheDocument()
    view.unmount()
  })

  it('keeps one live job channel through Strict Mode effect replay', async () => {
    const binding = vi.spyOn(commands, 'subscribe').mockResolvedValue(startupState())
    const view = render(
      createElement(StrictMode, null, createElement(Providers, null, createElement('div'))),
    )

    await waitFor(() => expect(binding).toHaveBeenCalledTimes(1))
    const [, jobChannel] = binding.mock.calls[0]
    act(() => {
      jobChannel.onmessage({
        id: 'job',
        kind: 'pipeline',
        phase: { kind: 'pipeline', stage: 'detection' },
        state: 'running',
        completed: 0,
        total: 4,
        page: 'page',
        error: null,
      })
    })

    expect(useKoharuStore.getState().jobs.job).toMatchObject({ state: 'running', total: 4 })
    view.unmount()
  })

  it('passes only domain arguments to mutation commands', async () => {
    const rename = vi.spyOn(commands, 'renamePage').mockResolvedValue(null)

    await expect(call(commands.renamePage, 'page', 'Chapter 1')).resolves.toBeNull()
    expect(rename).toHaveBeenCalledWith('page', 'Chapter 1')
  })

  it('passes the managed project name to open', async () => {
    const open = vi.spyOn(commands, 'openProject').mockResolvedValue(null)
    await expect(call(commands.openProject, 'Volume 1')).resolves.toBeNull()
    expect(open).toHaveBeenCalledWith('Volume 1')
  })

  it('applies independent channel updates directly to the store', async () => {
    useKoharuStore.setState({ downloads: {}, resources: null })
    const { binding, dispose } = await start()
    const [, , downloadChannel, resourcesChannel] = binding.mock.calls[0]

    downloadChannel.onmessage({
      id: 7,
      state: 'running',
      name: 'model.bin',
      completed: 25,
      total: 100,
      error: null,
    })
    resourcesChannel.onmessage({
      process_memory: 1024,
      system_memory: 8192,
      process_cpu: 5,
      devices: [
        { name: 'GPU', selected: true, memory_budget: 8192, memory_used: 4096, utilization: 40 },
      ],
    })
    expect(useKoharuStore.getState().downloads[7]).toMatchObject({ completed: 25, total: 100 })
    expect(useKoharuStore.getState().resources).toMatchObject({
      process_cpu: 5,
      devices: [{ memory_used: 4096 }],
    })
    dispose()
  })

  it('refreshes project queries when an autonomous job commits work', async () => {
    vi.spyOn(commands, 'getProject').mockResolvedValueOnce(null).mockResolvedValue(project)
    const binding = vi.spyOn(commands, 'subscribe').mockResolvedValue(startupState())
    const view = render(createElement(Providers, null, createElement(ProjectProbe)))
    expect(await screen.findByText('Closed')).toBeInTheDocument()

    const [, jobChannel] = binding.mock.calls[0]
    jobChannel.onmessage({
      id: 'job',
      kind: 'pipeline',
      phase: { kind: 'pipeline', stage: 'ocr' },
      state: 'running',
      completed: 1,
      total: 2,
      page: 'page',
      error: null,
    })

    expect(await screen.findByText('Book')).toBeInTheDocument()
    view.unmount()
  })

  it('does not refetch glossary for progress and refetches on terminal glossary transition', async () => {
    vi.spyOn(commands, 'getProject').mockResolvedValue(project)
    const getGlossary = vi.spyOn(commands, 'getGlossary').mockResolvedValue(glossary)
    const binding = vi.spyOn(commands, 'subscribe').mockResolvedValue(startupState())
    const view = render(createElement(Providers, null, createElement(GlossaryProbe)))
    expect(await screen.findByText('Glossary 3')).toBeInTheDocument()
    expect(getGlossary).toHaveBeenCalledOnce()

    const [, jobChannel] = binding.mock.calls[0]
    act(() => {
      jobChannel.onmessage({
        id: 'pipeline',
        kind: 'pipeline',
        phase: { kind: 'pipeline', stage: 'ocr' },
        state: 'running',
        completed: 1,
        total: 3,
        page: 'page',
        error: null,
      })
      jobChannel.onmessage({
        id: 'glossary',
        kind: 'glossary_scan',
        phase: { kind: 'extracting_terms' },
        state: 'running',
        completed: 1,
        total: 3,
        page: null,
        error: null,
      })
      jobChannel.onmessage({
        id: 'glossary',
        kind: 'glossary_scan',
        phase: { kind: 'extracting_terms' },
        state: 'running',
        completed: 2,
        total: 3,
        page: null,
        error: null,
      })
    })
    await act(async () => undefined)
    expect(getGlossary).toHaveBeenCalledOnce()

    act(() => {
      jobChannel.onmessage({
        id: 'glossary',
        kind: 'glossary_scan',
        phase: { kind: 'extracting_terms' },
        state: 'finished',
        completed: 3,
        total: 3,
        page: null,
        error: null,
      })
    })
    await waitFor(() => expect(getGlossary).toHaveBeenCalledTimes(2))
    expect(queryClient.getQueryData(glossaryKey(project.name))).toEqual(glossary)
    view.unmount()
  })

  it('observes startup glossary jobs before their first terminal event', async () => {
    let currentProject = project
    const activeJob: Job = {
      id: 'startup-glossary',
      kind: 'glossary_translation',
      phase: { kind: 'translating_terms' },
      state: 'running',
      completed: 1,
      total: 3,
      page: null,
      error: null,
    }
    vi.spyOn(commands, 'getProject').mockImplementation(async () => currentProject)
    const getGlossary = vi.spyOn(commands, 'getGlossary').mockResolvedValue(glossary)
    const binding = vi
      .spyOn(commands, 'subscribe')
      .mockResolvedValue({ ...startupState(), jobs: [activeJob] })
    const view = render(createElement(Providers, null, createElement(GlossaryProbe)))
    expect(await screen.findByText('Glossary 3')).toBeInTheDocument()
    expect(getGlossary).toHaveBeenCalledOnce()

    const [, jobChannel, , , projectChannel] = binding.mock.calls[0]
    act(() => {
      jobChannel.onmessage({ ...activeJob, state: 'finished', completed: 3 })
    })
    await waitFor(() => expect(getGlossary).toHaveBeenCalledTimes(2))

    act(() => {
      jobChannel.onmessage({
        id: 'pipeline-progress',
        kind: 'pipeline',
        phase: { kind: 'pipeline', stage: 'ocr' },
        state: 'running',
        completed: 1,
        total: 3,
        page: 'page',
        error: null,
      })
      jobChannel.onmessage({
        id: 'glossary-progress',
        kind: 'glossary_scan',
        phase: { kind: 'extracting_terms' },
        state: 'running',
        completed: 1,
        total: 3,
        page: null,
        error: null,
      })
      jobChannel.onmessage({
        id: 'old-project-glossary',
        kind: 'glossary_scan',
        phase: { kind: 'extracting_terms' },
        state: 'running',
        completed: 0,
        total: 2,
        page: null,
        error: null,
      })
    })
    await act(async () => undefined)
    expect(getGlossary).toHaveBeenCalledTimes(2)

    currentProject = { ...project, name: 'Other Book' }
    act(() => {
      projectChannel.onmessage(currentProject)
    })
    await waitFor(() => expect(getGlossary.mock.calls.length).toBeGreaterThan(2))
    const callsAfterSwitch = getGlossary.mock.calls.length

    act(() => {
      jobChannel.onmessage({
        id: 'old-project-glossary',
        kind: 'glossary_scan',
        phase: { kind: 'extracting_terms' },
        state: 'finished',
        completed: 2,
        total: 2,
        page: null,
        error: null,
      })
      jobChannel.onmessage({
        id: 'unrelated-terminal',
        kind: 'pipeline',
        phase: { kind: 'pipeline', stage: 'translation' },
        state: 'finished',
        completed: 1,
        total: 1,
        page: 'page',
        error: null,
      })
    })
    await act(async () => undefined)
    expect(getGlossary).toHaveBeenCalledTimes(callsAfterSwitch)
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
