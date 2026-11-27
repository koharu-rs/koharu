import { QueryClientProvider } from '@tanstack/react-query'
import { act, render as testingRender, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { ReactNode } from 'react'
import { describe, expect, it, vi } from 'vitest'

import { ActivityCenter } from '@/components/editor/ActivityCenter'
import { RightSidebar } from '@/components/editor/RightSidebar'
import { StatusBar } from '@/components/editor/StatusBar'
import { pageKey, pagesKey, projectKey, queryClient } from '@/lib/queries'
import { receiveDownload, receivePipeline, useKoharuStore } from '@/lib/store'
import { commands } from '@koharu/ipc/protocol'

function render(ui: ReactNode) {
  return testingRender(<QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>)
}

describe('ActivityCenter, RightSidebar Activity tab, and StatusBar pill', () => {
  it('renders empty state when idle and switches to Activity tab from StatusBar pill', async () => {
    queryClient.setQueryData(pageKey, {
      id: 'p1',
      size: { width: 800, height: 1200 },
      layers: [],
    })
    const user = userEvent.setup()
    render(
      <>
        <RightSidebar />
        <StatusBar onZoomChange={vi.fn()} />
      </>,
    )

    expect(screen.getByRole('tab', { name: 'Activity' })).toBeInTheDocument()
    expect(screen.queryByTestId('activity-tab-badge')).not.toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Activity' }))
    expect(useKoharuStore.getState().panel).toBe('activity')
    expect(await screen.findByText('No recent activity')).toBeInTheDocument()
  })

  it('shows active badge, running job with per-run stop, queued runs in order, and uncollapsed downloads with cancel/retry/dismiss', async () => {
    queryClient.setQueryData(projectKey, {
      name: 'Book',
      page: 'p1',
      pages: [
        { id: 'p1', size: { width: 800, height: 1200 }, texts: 0, translated: 0 },
        { id: 'p2', size: { width: 800, height: 1200 }, texts: 0, translated: 0 },
      ],
      history: { steps: [], current: 0 },
      revisions: {},
    })
    queryClient.setQueryData(pagesKey, [
      { id: 'p1', size: { width: 800, height: 1200 }, texts: 0, translated: 0 },
      { id: 'p2', size: { width: 800, height: 1200 }, texts: 0, translated: 0 },
    ])
    queryClient.setQueryData(pageKey, {
      id: 'p1',
      size: { width: 800, height: 1200 },
      layers: [],
    })

    const cancelRunSpy = vi.spyOn(commands, 'cancelRun').mockResolvedValue(null)
    const cancelDlSpy = vi.spyOn(commands, 'cancelDownload').mockResolvedValue(null)
    const retryDlSpy = vi.spyOn(commands, 'retryDownload').mockResolvedValue(null)
    const dismissDlSpy = vi.spyOn(commands, 'dismissDownload').mockResolvedValue(null)

    useKoharuStore.setState({
      panel: 'activity',
      loading: { loaded: 2, total: 5, error: null },
    })

    act(() => {
      receivePipeline({
        kind: 'queued',
        run: 1,
        steps: 4,
        priority: 'normal',
        scope: { kind: 'project' },
        stages: ['detect', 'inpaint'],
        jobs: [
          { page: 'p1', stage: 'detect' },
          { page: 'p2', stage: 'detect' },
          { page: 'p1', stage: 'inpaint' },
          { page: 'p2', stage: 'inpaint' },
        ],
      })
      receivePipeline({
        kind: 'started',
        run: 1,
        page: 'p1',
        stage: 'detect',
        model: 'koharu-layout-highres',
      })
      receivePipeline({
        kind: 'queued',
        run: 2,
        steps: 2,
        priority: 'rerun_now',
        scope: { kind: 'pages', count: 2 },
        stages: ['inpaint'],
        jobs: [
          { page: 'p1', stage: 'inpaint' },
          { page: 'p2', stage: 'inpaint' },
        ],
      })
      receivePipeline({
        kind: 'queued',
        run: 3,
        steps: 1,
        priority: 'normal',
        scope: { kind: 'metadata' },
        stages: ['translate'],
        jobs: [{ page: null, stage: 'translate' }],
      })
      receivePipeline({ kind: 'waiting_for_download', run: 3, downloads: [10] })
      receivePipeline({
        kind: 'queue_updated',
        order: [
          {
            run: 2,
            steps: 2,
            scope: { kind: 'pages', count: 2 },
            stages: ['inpaint'],
            jobs: [
              { page: 'p1', stage: 'inpaint' },
              { page: 'p2', stage: 'inpaint' },
            ],
            priority: 'rerun_now',
            waiting_downloads: [],
            reason: 'grouped_by_loaded_model',
          },
          {
            run: 3,
            steps: 1,
            scope: { kind: 'metadata' },
            stages: ['translate'],
            jobs: [{ page: null, stage: 'translate' }],
            priority: 'normal',
            waiting_downloads: [10],
            reason: 'waiting_for_download',
          },
        ],
      })
      receiveDownload({
        id: 10,
        model_id: 'translate:koharu/extremely-long-model-name-that-must-truncate',
        model_name: 'Extremely Long Quantized Translation Model Name For Truncation Check',
        stage: 'translate',
        completed: 500_000_000,
        total: 1_000_000_000,
        bytes_per_sec: 12_500_000,
        files_done: 1,
        files_total: 2,
        state: 'Running',
      })
      receiveDownload({
        id: 11,
        model_id: 'inpaint:flux3-klein',
        model_name: 'FLUX.3 Klein',
        stage: 'inpaint',
        completed: 200_000_000,
        total: 1_000_000_000,
        bytes_per_sec: 0,
        files_done: 0,
        files_total: 3,
        state: { Failed: 'Connection reset' },
      })
      receiveDownload({
        id: 12,
        model_id: 'detect:koharu-layout-highres',
        model_name: 'Koharu Layout HighRes',
        stage: 'detect',
        completed: 100_000_000,
        total: 100_000_000,
        bytes_per_sec: 0,
        files_done: 2,
        files_total: 2,
        state: 'Finished',
      })
    })

    const user = userEvent.setup()
    render(
      <>
        <RightSidebar />
        <StatusBar onZoomChange={vi.fn()} />
      </>,
    )

    // Badge counts active jobs (3) + running downloads (1) + chapter (1) = 5.
    expect(screen.getByTestId('activity-tab-badge')).toHaveTextContent('5')

    // Running section: chapter + running job with model badge & per-run Stop button.
    expect(screen.getByText('Loading pages: 2 of 5')).toBeInTheDocument()
    expect(screen.getByText('koharu-layout-highres')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Stop run' }))
    expect(cancelRunSpy).toHaveBeenCalledWith(1, null, null)

    // Queued section: active run's remaining steps in stage-batched order + queued runs + per-step Cancel.
    expect(screen.getByText('Page 2 Detection')).toBeInTheDocument()
    expect(screen.getAllByText('Page 1 Inpainting')).toHaveLength(2)
    expect(screen.getAllByText('Page 2 Inpainting')).toHaveLength(2)
    expect(screen.getByText('Metadata Translation')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Cancel Page 2 Detection' }))
    expect(cancelRunSpy).toHaveBeenCalledWith(1, 'p2', 'detect')
    await user.click(screen.getByRole('button', { name: 'Cancel Metadata Translation' }))
    expect(cancelRunSpy).toHaveBeenCalledWith(3, null, 'translate')

    expect(screen.getByText('#1')).toBeInTheDocument()
    expect(screen.getByText('#2')).toBeInTheDocument()
    expect(screen.getByText('Rerun now')).toBeInTheDocument()
    expect(screen.getByText('Waiting for download')).toBeInTheDocument()
    expect(screen.getByText('Grouped by loaded model')).toBeInTheDocument()
    expect(screen.getByText('Waiting for model download')).toBeInTheDocument()
    const cancelQueuedButtons = screen.getAllByRole('button', { name: 'Cancel queued run' })
    expect(cancelQueuedButtons).toHaveLength(2)
    await user.click(cancelQueuedButtons[0])
    expect(cancelRunSpy).toHaveBeenCalledWith(2, null, null)

    // Uncollapsed Downloads section: all 3 downloads rendered individually (never grouped).
    expect(
      screen.getByText('Extremely Long Quantized Translation Model Name For Truncation Check'),
    ).toBeInTheDocument()
    expect(screen.getByText('FLUX.3 Klein')).toBeInTheDocument()
    expect(screen.getByText('Koharu Layout HighRes')).toBeInTheDocument()
    expect(screen.getByText('Connection reset')).toBeInTheDocument()
    expect(screen.getByText(/12\.5 MB\/s/)).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Cancel download' }))
    expect(cancelDlSpy).toHaveBeenCalledWith(10)

    await user.click(screen.getByRole('button', { name: 'Retry download' }))
    expect(retryDlSpy).toHaveBeenCalledWith(11)

    const dismissButtons = screen.getAllByRole('button', { name: 'Dismiss download' })
    expect(dismissButtons).toHaveLength(2)
    await user.click(dismissButtons[0])
    await waitFor(() => expect(dismissDlSpy).toHaveBeenCalledWith(11))
    expect(useKoharuStore.getState().downloads[11]).toBeUndefined()
  })

  it('keeps ActivityCenter standalone rendering clean when all downloads are finished', () => {
    act(() => {
      receiveDownload({
        id: 20,
        model_id: 'lama',
        model_name: 'LaMa',
        stage: 'inpaint',
        completed: 50_000_000,
        total: 50_000_000,
        bytes_per_sec: 0,
        files_done: 1,
        files_total: 1,
        state: 'Finished',
      })
    })
    render(<ActivityCenter />)
    expect(screen.getByText('LaMa')).toBeInTheDocument()
    expect(screen.getByText(/Downloaded/)).toBeInTheDocument()
  })
})
