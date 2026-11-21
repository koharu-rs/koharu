'use client'

import { create } from 'zustand'

import type {
  Bounds,
  LayerId,
  PageId,
  PipelineEvent,
  RunId,
  Size,
  Stage,
  Tool,
  Transfer,
  Typed,
} from '@koharu/ipc/protocol'

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
export type Panel = 'layers' | 'story' | 'agent'
export type PipelineScope = 'selected-pages' | 'project'
export const pipelineStages: readonly Stage[] = ['detect', 'recognize', 'inpaint', 'translate']

/** A pipeline run as the activity center shows it, from the pipeline's events. */
export interface Job {
  id: RunId
  state: 'running' | 'done' | 'failed' | 'stopped'
  /** The page stages done, of `total`. */
  completed: number
  total: number
  /** The stage started last, and its page. */
  stage: Stage | null
  page: PageId | null
  error: string | null
}

interface KoharuStore {
  initialized: boolean
  jobs: Record<string, Job>
  downloads: Record<string, Transfer>
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
  /** Whether the History window shows over the canvas. */
  historyOpen: boolean
  selectPages: (pages: PageId[]) => void
  showPanel: (panel: Panel) => void
  setStoryHover: (layer: LayerId | null) => void
  setProcessingScope: (scope: PipelineScope) => void
  setProcessingStages: (stages: Stage[]) => void
  setSettingsOpen: (open: boolean) => void
  setHistoryOpen: (open: boolean) => void
  selectLayers: (layers: LayerId[]) => void
  setTool: (tool: CanvasTool) => void
  setBrush: (brush: CanvasBrush) => void
  dismissJob: (id: string) => void
  dismissDownload: (id: number) => void
}

export const useKoharuStore = create<KoharuStore>()((set) => ({
  initialized: false,
  jobs: {},
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
  historyOpen: false,
  selectPages: (selectedPages) => set({ selectedPages: [...new Set(selectedPages)] }),
  showPanel: (panel) => set({ panel, storyHover: null }),
  setStoryHover: (storyHover) => set({ storyHover }),
  setProcessingScope: (processingScope) => set({ processingScope }),
  setProcessingStages: (processingStages) => set({ processingStages }),
  setSettingsOpen: (settingsOpen) => set({ settingsOpen }),
  setHistoryOpen: (historyOpen) => set({ historyOpen }),
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
}))

/** Every run's events so far, as the editor loads. */
export function receiveRuns(events: PipelineEvent[]): void {
  const jobs: Record<string, Job> = {}
  for (const event of events) jobs[String(event.run)] = fold(jobs[String(event.run)], event)
  useKoharuStore.setState({ initialized: true, jobs })
}

export function receivePipeline(event: PipelineEvent): void {
  useKoharuStore.setState((state) => ({
    jobs: { ...state.jobs, [String(event.run)]: fold(state.jobs[String(event.run)], event) },
  }))
}

function fold(job: Job | undefined, event: PipelineEvent): Job {
  const current: Job = job ?? {
    id: event.run,
    state: 'running',
    completed: 0,
    total: 0,
    stage: null,
    page: null,
    error: null,
  }
  switch (event.kind) {
    case 'queued':
      return { ...current, total: event.steps }
    case 'started':
      return { ...current, stage: event.stage, page: event.page }
    case 'finished':
      return { ...current, completed: current.completed + 1 }
    case 'ended':
      // A failure names its own page and stage: the last started may be another page in flight.
      return event.outcome.kind === 'failed'
        ? {
            ...current,
            state: 'failed',
            stage: event.outcome.stage,
            page: event.outcome.page,
            error: event.outcome.reason,
          }
        : { ...current, state: event.outcome.kind, error: null }
  }
}

export function receiveDownload(download: Transfer): void {
  useKoharuStore.setState((state) => {
    const downloads = { ...state.downloads }
    if (download.state === 'Finished') delete downloads[String(download.id)]
    else downloads[String(download.id)] = download
    return { downloads }
  })
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
