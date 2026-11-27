'use client'

import { create } from 'zustand'
import { persist } from 'zustand/middleware'

import {
  commands,
  type Bounds,
  type DownloadEntry,
  type LayerId,
  type Loading,
  type PageId,
  type PipelineEvent,
  type QueueScope,
  type QueuedRunSummary,
  type QueuedStep,
  type RunId,
  type RunPriority,
  type Size,
  type Stage,
  type Tool,
  type Typed,
} from '@koharu/ipc/protocol'

import { send } from './queries'

export interface FileDragTarget {
  x: number
  y: number
  target: 'rail' | 'workspace'
  insertionIndex: number | null
  /** Y offset in CSS px inside `[data-page-rail-body]`, used when `target === 'rail'`. */
  lineTop: number
}

export type CanvasTool = Tool
export const MIN_BRUSH_DIAMETER = 1
export function maxBrushDiameter(size: Size): number {
  // Assume that for all sizes below 2048 width, 128 is sufficient.
  // For anything above, scale by percentage greater than 2048 width.
  return Math.round(Math.max(size.width / 2048, 1.0) * 128)
}

export interface CanvasBrush {
  diameter: number
  color: string
}

/** Where the page shows: a page point `p` sits at `p * zoom + (x, y)` in CSS pixels from the
 * canvas element's corner, as the native canvas publishes it. */
export interface Camera {
  zoom: number
  x: number
  y: number
}

/** The right pane's tabs. */
export type Panel = 'layers' | 'story' | 'agent' | 'activity'
export type PipelineScope = 'selected-pages' | 'project'
export const pipelineStages: readonly Stage[] = ['detect', 'recognize', 'inpaint', 'translate']

/** A pipeline run as the activity center shows it, from the pipeline's events. */
export interface Job {
  id: RunId
  state: 'queued' | 'waiting_for_download' | 'running' | 'done' | 'failed' | 'stopped' | 'coalesced'
  /** The page stages done, of `total`. */
  completed: number
  total: number
  /** The stage started last, and its page. */
  stage: Stage | null
  page: PageId | null
  model: string | null
  priority: RunPriority | null
  scope: QueueScope | null
  stages: Stage[]
  /** Remaining unstarted page-stage steps in stage-batched execution order. */
  pendingSteps: QueuedStep[]
  /** Steps currently executing (can hold multiple pages during batched OCR). */
  runningSteps: QueuedStep[]
  waitingDownloads: number[]
  error: string | null
  /** The pages a stage failed on. */
  failed: PageId[]
}

export function isActiveJob(job: Job): boolean {
  return job.state === 'running' || job.state === 'queued' || job.state === 'waiting_for_download'
}

interface KoharuStore {
  initialized: boolean
  jobs: Record<string, Job>
  queueOrder: QueuedRunSummary[]
  downloads: Record<string, DownloadEntry>
  camera: Camera
  /** Where the canvas's selection lies on the page as it's laid out, a point text's lines
   * included. */
  selectionBounds: Bounds | null
  /** Where the area the canvas's marquee and lasso marked lies on the page. */
  area: Bounds | null
  /** The text the canvas is typing into, and its selected characters' style. */
  typing: Typed | null
  /** The colour the canvas's picker picked last, numbered so a repeat counts. */
  picked: { color: string; serial: number } | null
  selectedLayers: LayerId[]
  /** The selection the canvas has, so one it published isn't sent back to it. */
  canvasSelection: LayerId[]
  selectedPages: PageId[]
  tool: CanvasTool
  brush: CanvasBrush
  /** The right pane's tab, which the canvas numbers the texts for while it's Story. */
  panel: Panel
  /** The text whose Story entry the pointer is over, outlined on the canvas. */
  storyHover: LayerId | null
  processingScope: PipelineScope
  processingStages: Stage[]
  settingsOpen: boolean
  /** Whether the ComicInfo metadata dialog is open, never persisted. */
  metadataOpen: boolean
  /** Whether the canvas shows the page's scan alone. */
  original: boolean
  /** Whether the History window shows over the canvas. */
  historyOpen: boolean
  /** Whether the start view shows the gallery, not the projects. */
  galleryOpen: boolean
  /** Whether the main window reads the open project, not edits it. */
  reading: boolean
  /** How far an extension's chapter loading into the open project went. */
  loading: Loading | null
  /** Active OS file/folder drag coordinates and resolved drop target, never persisted. */
  fileDrag: FileDragTarget | null
  selectPages: (pages: PageId[]) => void
  showPanel: (panel: Panel) => void
  setStoryHover: (layer: LayerId | null) => void
  setProcessingScope: (scope: PipelineScope) => void
  setProcessingStages: (stages: Stage[]) => void
  setSettingsOpen: (open: boolean) => void
  setMetadataOpen: (open: boolean) => void
  setOriginal: (original: boolean) => void
  setHistoryOpen: (open: boolean) => void
  setGalleryOpen: (open: boolean) => void
  setReading: (reading: boolean) => void
  selectLayers: (layers: LayerId[]) => void
  setTool: (tool: CanvasTool) => void
  setBrush: (brush: CanvasBrush) => void
  dismissJob: (id: string) => void
  dismissDownload: (id: number) => void
}

export const useKoharuStore = create<KoharuStore>()(
  persist(
    (set) => ({
      initialized: false,
      jobs: {},
      queueOrder: [],
      downloads: {},
      camera: { zoom: 1, x: 0, y: 0 },
      selectionBounds: null,
      area: null,
      typing: null,
      picked: null,
      selectedLayers: [],
      canvasSelection: [],
      selectedPages: [],
      tool: 'select',
      brush: { diameter: 48, color: '#FFFFFF' },
      panel: 'layers',
      storyHover: null,
      processingScope: 'project',
      processingStages: [...pipelineStages],
      settingsOpen: false,
      metadataOpen: false,
      original: false,
      historyOpen: false,
      galleryOpen: false,
      reading: false,
      loading: null,
      fileDrag: null,
      selectPages: (selectedPages) => set({ selectedPages: [...new Set(selectedPages)] }),
      showPanel: (panel) => set({ panel, storyHover: null }),
      setStoryHover: (storyHover) => set({ storyHover }),
      setProcessingScope: (processingScope) => set({ processingScope }),
      setProcessingStages: (processingStages) => set({ processingStages }),
      setSettingsOpen: (settingsOpen) => set({ settingsOpen }),
      setMetadataOpen: (metadataOpen) => set({ metadataOpen }),
      setOriginal: (original) => set({ original }),
      setHistoryOpen: (historyOpen) => set({ historyOpen }),
      setGalleryOpen: (galleryOpen) => set({ galleryOpen }),
      setReading: (reading) => set({ reading }),
      selectLayers: (selectedLayers) => set({ selectedLayers: [...new Set(selectedLayers)] }),
      setTool: (tool) => set({ tool }),
      setBrush: (brush) => set({ brush }),
      dismissJob: (id) =>
        set((state) => {
          const jobs = { ...state.jobs }
          delete jobs[id]
          return { jobs }
        }),
      dismissDownload: (id) =>
        set((state) => {
          const downloads = { ...state.downloads }
          delete downloads[String(id)]
          return { downloads }
        }),
    }),
    {
      name: 'koharu.workspace',
      // The choices a user makes once and expects again next time, as Photoshop keeps its tool
      // and brush; what's selected, running, or open starts afresh.
      partialize: ({ tool, brush, processingScope, processingStages, galleryOpen }) => ({
        tool,
        brush,
        processingScope,
        processingStages,
        galleryOpen,
      }),
    },
  ),
)

/** Every run's events so far, as the editor loads. */
export function receiveRuns(events: PipelineEvent[]): void {
  let state: PipelineSlice = { jobs: {}, queueOrder: [] }
  for (const event of events) state = reducePipelineState(state, event)
  useKoharuStore.setState({ initialized: true, jobs: state.jobs, queueOrder: state.queueOrder })
}

export function receivePipeline(event: PipelineEvent): void {
  useKoharuStore.setState((state) => reducePipelineState(state, event))
}

interface PipelineSlice {
  jobs: Record<string, Job>
  queueOrder: QueuedRunSummary[]
}

/** `job` with a step a stage failed on, which shows once the run ends. */
function failed(job: Job, { page, reason }: { page: PageId | null; reason: string }): Job {
  return { ...job, error: reason, failed: page !== null ? [...job.failed, page] : job.failed }
}

function sameStep(a: QueuedStep, b: { page: PageId | null; stage: Stage }): boolean {
  return a.page === b.page && a.stage === b.stage
}

function emptyJob(run: RunId): Job {
  return {
    id: run,
    state: 'queued',
    completed: 0,
    total: 0,
    stage: null,
    page: null,
    model: null,
    priority: null,
    scope: null,
    stages: [],
    pendingSteps: [],
    runningSteps: [],
    waitingDownloads: [],
    error: null,
    failed: [],
  }
}

function reducePipelineState(state: PipelineSlice, event: PipelineEvent): PipelineSlice {
  if (event.kind === 'queue_updated') {
    const jobs = { ...state.jobs }
    for (const item of event.order) {
      const key = String(item.run)
      const existing = jobs[key]
      if (existing && (existing.state === 'queued' || existing.state === 'waiting_for_download')) {
        jobs[key] = {
          ...existing,
          total: item.steps,
          scope: item.scope,
          stages: item.stages,
          pendingSteps: item.jobs,
          priority: item.priority,
          waitingDownloads: item.waiting_downloads,
          state: item.waiting_downloads.length > 0 ? 'waiting_for_download' : 'queued',
        }
      }
    }
    return { jobs, queueOrder: event.order }
  }

  const key = String(event.run)
  const current = state.jobs[key] ?? emptyJob(event.run)

  switch (event.kind) {
    case 'queued':
      return {
        ...state,
        jobs: {
          ...state.jobs,
          [key]: {
            ...current,
            state: 'queued',
            total: event.steps,
            priority: event.priority,
            scope: event.scope,
            stages: event.stages,
            pendingSteps: event.jobs,
            runningSteps: [],
            waitingDownloads: [],
          },
        },
      }
    case 'waiting_for_download':
      return {
        ...state,
        jobs: {
          ...state.jobs,
          [key]: {
            ...current,
            state: 'waiting_for_download',
            waitingDownloads: event.downloads,
          },
        },
      }
    case 'started': {
      const step: QueuedStep = { page: event.page, stage: event.stage }
      const pendingSteps = current.pendingSteps.filter((candidate) => !sameStep(candidate, step))
      const runningSteps = current.runningSteps.some((candidate) => sameStep(candidate, step))
        ? current.runningSteps
        : [...current.runningSteps, step]
      return {
        queueOrder: state.queueOrder.filter((item) => item.run !== event.run),
        jobs: {
          ...state.jobs,
          [key]: {
            ...current,
            state: 'running',
            stage: event.stage,
            page: event.page,
            model: event.model ?? current.model,
            pendingSteps,
            runningSteps,
            waitingDownloads: [],
          },
        },
      }
    }
    case 'finished': {
      const runningSteps = current.runningSteps.filter((candidate) => !sameStep(candidate, event))
      const pendingSteps = current.pendingSteps.filter((candidate) => !sameStep(candidate, event))
      const lead = runningSteps[0]
      return {
        ...state,
        jobs: {
          ...state.jobs,
          [key]: {
            ...current,
            completed: current.completed + 1,
            stage: lead?.stage ?? current.stage,
            page: lead !== undefined ? lead.page : current.page,
            runningSteps,
            pendingSteps,
          },
        },
      }
    }
    case 'failed': {
      const runningSteps = current.runningSteps.filter((candidate) => !sameStep(candidate, event))
      const pendingSteps = current.pendingSteps.filter((candidate) => !sameStep(candidate, event))
      const lead = runningSteps[0]
      return {
        ...state,
        jobs: {
          ...state.jobs,
          [key]: {
            ...failed(current, event),
            completed: current.completed + 1,
            stage: lead?.stage ?? current.stage,
            page: lead !== undefined ? lead.page : current.page,
            runningSteps,
            pendingSteps,
          },
        },
      }
    }
    case 'job_cancelled': {
      const runningSteps = current.runningSteps.filter((candidate) => !sameStep(candidate, event))
      const pendingSteps = current.pendingSteps.filter((candidate) => !sameStep(candidate, event))
      const lead = runningSteps[0]
      return {
        queueOrder: state.queueOrder.map((item) => {
          if (item.run !== event.run) return item
          const jobs = item.jobs.filter((candidate) => !sameStep(candidate, event))
          return { ...item, steps: jobs.length, jobs }
        }),
        jobs: {
          ...state.jobs,
          [key]: {
            ...current,
            total: Math.max(current.completed, current.total - 1),
            stage: lead?.stage ?? current.stage,
            page: lead !== undefined ? lead.page : current.page,
            runningSteps,
            pendingSteps,
          },
        },
      }
    }
    case 'coalesced':
      return {
        queueOrder: state.queueOrder.filter((item) => item.run !== event.run),
        jobs: {
          ...state.jobs,
          [key]: {
            ...current,
            state: 'coalesced',
            pendingSteps: [],
            runningSteps: [],
            waitingDownloads: [],
          },
        },
      }
    case 'ended': {
      const { outcome } = event
      const ended = outcome.kind === 'failed' ? failed(current, outcome) : current
      return {
        queueOrder: state.queueOrder.filter((item) => item.run !== event.run),
        jobs: {
          ...state.jobs,
          [key]: {
            ...ended,
            pendingSteps: [],
            runningSteps: [],
            waitingDownloads: [],
            state: ended.failed.length > 0 || ended.error !== null ? 'failed' : outcome.kind,
          },
        },
      }
    }
  }
}

export function receiveDownload(download: DownloadEntry): void {
  useKoharuStore.setState((state) => ({
    downloads: { ...state.downloads, [String(download.id)]: download },
  }))
}

export function receiveDownloads(entries: DownloadEntry[]): void {
  useKoharuStore.setState((state) => ({
    downloads: {
      ...Object.fromEntries(entries.map((entry) => [String(entry.id), entry])),
      ...state.downloads,
    },
  }))
}

export function dismissDownloadFromDesktop(id: number): void {
  useKoharuStore.getState().dismissDownload(id)
  send(commands.dismissDownload, id)
}

/** A selection the canvas made. */
export function receiveCanvasSelection(layers: LayerId[]): void {
  useKoharuStore.setState({ selectedLayers: [...new Set(layers)], canvasSelection: layers })
}

export function receivePickedColor(color: string): void {
  useKoharuStore.setState((state) => ({
    picked: { color, serial: (state.picked?.serial ?? 0) + 1 },
  }))
}
