import { waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import {
  dismissDownloadFromDesktop,
  isActiveJob,
  receiveDownload,
  receiveDownloads,
  receivePipeline,
  receiveRuns,
  useKoharuStore,
} from '@/lib/store'
import { commands } from '@koharu/ipc/protocol'

describe('editor state', () => {
  it('retains failed, finished, and cancelled downloads until dismissed', async () => {
    useKoharuStore.setState({ downloads: {} })
    receiveDownload({
      id: 9,
      model_id: 'weights.bin',
      model_name: 'weights.bin',
      stage: null,
      completed: 0,
      total: 0,
      bytes_per_sec: 0,
      files_done: 0,
      files_total: 1,
      state: { Failed: 'network unavailable' },
    })
    expect(useKoharuStore.getState().downloads[9]).toMatchObject({
      state: { Failed: 'network unavailable' },
      model_name: 'weights.bin',
    })

    useKoharuStore.getState().dismissDownload(9)
    expect(useKoharuStore.getState().downloads[9]).toBeUndefined()

    receiveDownload({
      id: 10,
      model_id: 'm1',
      model_name: 'Model 1',
      stage: 'detect',
      completed: 50,
      total: 100,
      bytes_per_sec: 100,
      files_done: 0,
      files_total: 1,
      state: 'Running',
    })
    receiveDownload({
      id: 10,
      model_id: 'm1',
      model_name: 'Model 1',
      stage: 'detect',
      completed: 100,
      total: 100,
      bytes_per_sec: 0,
      files_done: 1,
      files_total: 1,
      state: 'Finished',
    })
    expect(useKoharuStore.getState().downloads[10]).toMatchObject({ state: 'Finished' })

    receiveDownload({
      id: 11,
      model_id: 'm2',
      model_name: 'Model 2',
      stage: 'inpaint',
      completed: 50,
      total: 100,
      bytes_per_sec: 0,
      files_done: 0,
      files_total: 1,
      state: 'Cancelled',
    })
    expect(useKoharuStore.getState().downloads[11]).toMatchObject({ state: 'Cancelled' })

    const dismissSpy = vi.spyOn(commands, 'dismissDownload').mockResolvedValue(null)
    dismissDownloadFromDesktop(10)
    expect(useKoharuStore.getState().downloads[10]).toBeUndefined()
    await waitFor(() => expect(dismissSpy).toHaveBeenCalledWith(10))
  })

  it('hydrates downloads via receiveDownloads without clobbering newer live updates', () => {
    useKoharuStore.setState({
      downloads: {
        '2': {
          id: 2,
          model_id: 'm2',
          model_name: 'Model 2 Live',
          stage: 'inpaint',
          completed: 80,
          total: 100,
          bytes_per_sec: 2048,
          files_done: 0,
          files_total: 1,
          state: 'Running',
        },
      },
    })
    receiveDownloads([
      {
        id: 1,
        model_id: 'm1',
        model_name: 'Model 1 Hydrated',
        stage: 'detect',
        completed: 100,
        total: 100,
        bytes_per_sec: 0,
        files_done: 1,
        files_total: 1,
        state: 'Finished',
      },
      {
        id: 2,
        model_id: 'm2',
        model_name: 'Model 2 Stale',
        stage: 'inpaint',
        completed: 10,
        total: 100,
        bytes_per_sec: 100,
        files_done: 0,
        files_total: 1,
        state: 'Running',
      },
    ])
    expect(useKoharuStore.getState().downloads[1]?.model_name).toBe('Model 1 Hydrated')
    expect(useKoharuStore.getState().downloads[2]?.completed).toBe(80)
  })

  it('tracks waiting_for_download, queue_updated, coalesced, and panel activity', () => {
    useKoharuStore.getState().showPanel('activity')
    expect(useKoharuStore.getState().panel).toBe('activity')

    receiveRuns([
      {
        kind: 'queued',
        run: 1,
        steps: 2,
        priority: 'normal',
        scope: { kind: 'pages', count: 1 },
        stages: ['detect'],
        jobs: [{ page: 'p1', stage: 'detect' }],
      },
      {
        kind: 'queued',
        run: 2,
        steps: 2,
        priority: 'normal',
        scope: { kind: 'pages', count: 1 },
        stages: ['detect'],
        jobs: [{ page: 'p1', stage: 'detect' }],
      },
      { kind: 'waiting_for_download', run: 2, downloads: [5] },
      {
        kind: 'queue_updated',
        order: [
          {
            run: 1,
            steps: 4,
            scope: { kind: 'pages', count: 2 },
            stages: ['detect', 'inpaint'],
            jobs: [
              { page: 'p1', stage: 'detect' },
              { page: 'p2', stage: 'detect' },
              { page: 'p1', stage: 'inpaint' },
              { page: 'p2', stage: 'inpaint' },
            ],
            priority: 'normal',
            waiting_downloads: [],
            reason: 'grouped_by_loaded_model',
          },
          {
            run: 2,
            steps: 2,
            scope: { kind: 'pages', count: 1 },
            stages: ['detect'],
            jobs: [{ page: 'p1', stage: 'detect' }],
            priority: 'normal',
            waiting_downloads: [5],
            reason: 'waiting_for_download',
          },
        ],
      },
      { kind: 'coalesced', run: 2, into: 1 },
    ])

    const state = useKoharuStore.getState()
    expect(state.jobs[1]).toMatchObject({
      state: 'queued',
      total: 4,
      scope: { kind: 'pages', count: 2 },
    })
    expect(state.jobs[1]?.pendingSteps).toHaveLength(4)
    expect(isActiveJob(state.jobs[1])).toBe(true)
    expect(state.jobs[2]?.state).toBe('coalesced')
    expect(isActiveJob(state.jobs[2])).toBe(false)
    expect(state.queueOrder).toHaveLength(1)
    expect(state.queueOrder[0]?.run).toBe(1)

    receivePipeline({
      kind: 'started',
      run: 1,
      page: 'p1',
      stage: 'detect',
      model: 'koharu-layout-highres',
    })
    expect(useKoharuStore.getState().jobs[1]).toMatchObject({
      state: 'running',
      model: 'koharu-layout-highres',
      runningSteps: [{ page: 'p1', stage: 'detect' }],
    })
    expect(useKoharuStore.getState().jobs[1]?.pendingSteps).toHaveLength(3)
    expect(useKoharuStore.getState().queueOrder).toHaveLength(0)

    receivePipeline({
      kind: 'job_cancelled',
      run: 1,
      page: 'p2',
      stage: 'inpaint',
    })
    expect(useKoharuStore.getState().jobs[1]?.total).toBe(3)
    expect(useKoharuStore.getState().jobs[1]?.pendingSteps).toEqual([
      { page: 'p2', stage: 'detect' },
      { page: 'p1', stage: 'inpaint' },
    ])
  })

  it('defaults paint brush color to pure white (#FFFFFF)', () => {
    expect(useKoharuStore.getState().brush.color).toBe('#FFFFFF')
  })

  it('does not persist transient fileDrag coordinates to localStorage', () => {
    localStorage.removeItem('koharu.workspace')
    useKoharuStore.setState({
      fileDrag: { x: 40, y: 120, target: 'rail', insertionIndex: 2, lineTop: 88 },
    })
    const persisted = JSON.parse(localStorage.getItem('koharu.workspace') ?? '{}')
    expect(persisted.state?.fileDrag).toBeUndefined()
  })

  it('folds project-level metadata pipeline events (page: null) without putting null in failed', () => {
    useKoharuStore.setState({ jobs: {} })

    receivePipeline({
      kind: 'queued',
      run: 10,
      steps: 1,
      priority: 'normal',
      scope: { kind: 'metadata' },
      stages: ['translate'],
      jobs: [{ page: null, stage: 'translate' }],
    })
    receivePipeline({
      kind: 'started',
      run: 10,
      page: null,
      stage: 'translate',
      model: 'koharu/index-translate-9b',
    })
    expect(useKoharuStore.getState().jobs[10]).toMatchObject({
      state: 'running',
      stage: 'translate',
      page: null,
      model: 'koharu/index-translate-9b',
      total: 1,
    })

    receivePipeline({
      kind: 'failed',
      run: 10,
      page: null,
      stage: 'translate',
      reason: 'model refused',
    })
    receivePipeline({ kind: 'ended', run: 10, outcome: { kind: 'done' } })
    expect(useKoharuStore.getState().jobs[10]).toMatchObject({
      state: 'failed',
      completed: 1,
      error: 'model refused',
      failed: [],
    })

    receivePipeline({
      kind: 'queued',
      run: 11,
      steps: 1,
      priority: 'normal',
      scope: { kind: 'metadata' },
      stages: ['translate'],
      jobs: [{ page: null, stage: 'translate' }],
    })
    receivePipeline({
      kind: 'started',
      run: 11,
      page: null,
      stage: 'translate',
      model: null,
    })
    receivePipeline({
      kind: 'ended',
      run: 11,
      outcome: { kind: 'failed', page: null, stage: 'translate', reason: 'fatal error' },
    })
    expect(useKoharuStore.getState().jobs[11]).toMatchObject({
      state: 'failed',
      error: 'fatal error',
      failed: [],
    })
  })
})
