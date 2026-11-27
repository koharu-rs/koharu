import { QueryClientProvider } from '@tanstack/react-query'
import { act, render as testingRender, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { ReactNode } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { CanvasCommandBar } from '@/components/editor/CanvasCommandBar'
import { pageKey, preferencesKey, modelCatalogKey, queryClient } from '@/lib/queries'
import { receivePipeline, useKoharuStore } from '@/lib/store'
import { commands, type ModelCatalog, type Preferences } from '@koharu/ipc/protocol'

function render(ui: ReactNode) {
  return testingRender(<QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>)
}

const prefs: Preferences = {
  pipeline: { translator: { model: 'koharu/index-translate-9b', target: 'en', vision: false } },
  renderer: { styles: {} },
  agent: {},
  providers: {},
  export: { cbz_codec: 'png', quality: 90, lossless: false },
}

const catalog: ModelCatalog = {
  available_vram_bytes: null,
  total_vram_bytes: null,
  detect: [],
  recognize: [],
  inpaint: [],
  translate: [
    {
      reference: 'koharu/index-translate-9b',
      name: 'Index Translate 9B',
      vendor: 'koharu',
      images: false,
      tools: false,
      disk_bytes: 5_000_000_000,
      downloaded: true,
      measured_vram_bytes: null,
      variants: [],
    },
    {
      reference: 'koharu/gemma4-e2b-it',
      name: 'Gemma 4 E2B',
      vendor: 'koharu',
      images: true,
      tools: false,
      disk_bytes: 2_000_000_000,
      downloaded: true,
      measured_vram_bytes: null,
      variants: [],
    },
  ],
}

describe('CanvasCommandBar & InferenceControl concurrent queueing', () => {
  beforeEach(() => {
    queryClient.setQueryData(pageKey, {
      id: 'p1',
      size: { width: 800, height: 1200 },
      layers: [],
    })
    queryClient.setQueryData(preferencesKey, prefs)
    queryClient.setQueryData(modelCatalogKey, catalog)
    vi.spyOn(commands, 'listModelCatalog').mockResolvedValue(catalog)
    vi.spyOn(commands, 'getPreferences').mockResolvedValue(prefs)
  })

  it('submits normal when idle, unlocks Queue and Rerun now when active work exists, and keeps pickers unlocked', async () => {
    const runPipelineSpy = vi.spyOn(commands, 'runPipeline').mockResolvedValue(1)
    const user = userEvent.setup()

    render(<CanvasCommandBar />)

    // Idle state: single Run button submitting priority `'normal'`.
    const runBtn = screen.getByRole('button', { name: 'Run processing' })
    expect(runBtn).toBeEnabled()
    await user.click(runBtn)
    await waitFor(() =>
      expect(runPipelineSpy).toHaveBeenCalledWith(
        'project',
        ['detect', 'recognize', 'inpaint', 'translate'],
        'normal',
      ),
    )

    // Active run arrives -> Queue + Rerun now split control is shown and unlocked!
    act(() => {
      receivePipeline({
        kind: 'queued',
        run: 1,
        steps: 4,
        priority: 'normal',
        scope: { kind: 'project' },
        stages: ['detect', 'recognize', 'inpaint', 'translate'],
        jobs: [
          { page: 'p1', stage: 'detect' },
          { page: 'p1', stage: 'recognize' },
          { page: 'p1', stage: 'inpaint' },
          { page: 'p1', stage: 'translate' },
        ],
      })
      receivePipeline({
        kind: 'started',
        run: 1,
        page: 'p1',
        stage: 'detect',
        model: 'koharu-layout-highres',
      })
    })

    const queueBtn = await screen.findByRole('button', { name: 'Queue processing' })
    expect(queueBtn).toBeEnabled()
    await user.click(queueBtn)
    await waitFor(() => expect(runPipelineSpy).toHaveBeenCalledTimes(2))

    // Split dropdown offers Rerun now (`rerun_now`).
    await user.click(screen.getByRole('button', { name: 'Run options' }))
    await user.click(await screen.findByRole('menuitem', { name: /Rerun now/ }))
    await waitFor(() =>
      expect(runPipelineSpy).toHaveBeenLastCalledWith(
        'project',
        ['detect', 'recognize', 'inpaint', 'translate'],
        'rerun_now',
      ),
    )

    // ModelPicker and OutputPicker remain enabled while active work is running!
    await user.click(screen.getByRole('button', { name: 'Processing settings' }))
    await user.click(await screen.findByRole('button', { name: /^Model / }))
    expect(await screen.findByRole('option', { name: /Gemma 4 E2B/ })).not.toHaveAttribute(
      'aria-disabled',
      'true',
    )
    await user.click(screen.getByRole('button', { name: 'Back' }))
    await user.click(await screen.findByRole('button', { name: /^Output / }))
    expect(useKoharuStore.getState().jobs[1]?.state).toBe('running')
    expect(await screen.findByPlaceholderText('Search languages')).toBeEnabled()
  })
})
