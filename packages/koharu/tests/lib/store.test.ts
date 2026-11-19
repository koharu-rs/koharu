import { describe, expect, it } from 'vitest'

import { receiveDownload, receivePipeline, receiveRuns, useKoharuStore } from '@/lib/store'

describe('editor state', () => {
  it('shows the pending translation after concurrent inpainting ends', () => {
    receiveRuns([
      { kind: 'queued', run: 1, steps: 4 },
      { kind: 'started', run: 1, stage: 'translate', page: 'one' },
      { kind: 'started', run: 1, stage: 'translate', page: 'two' },
      { kind: 'started', run: 1, stage: 'inpaint', page: 'one' },
      { kind: 'finished', run: 1, stage: 'inpaint', page: 'one' },
      { kind: 'started', run: 1, stage: 'inpaint', page: 'two' },
      { kind: 'finished', run: 1, stage: 'inpaint', page: 'two' },
    ])
    const job = () => useKoharuStore.getState().jobs[1]
    expect(job()).toMatchObject({ completed: 2, total: 4, stage: 'translate', page: 'one' })
    receivePipeline({ kind: 'finished', run: 1, stage: 'translate', page: 'one' })
    expect(job()).toMatchObject({ completed: 3, stage: 'translate', page: 'two' })
    receivePipeline({ kind: 'finished', run: 1, stage: 'translate', page: 'two' })
    expect(job()).toMatchObject({ completed: 4, stage: null, page: null })
  })

  it('retains failed downloads until they are dismissed', () => {
    useKoharuStore.setState({ downloads: {} })
    receiveDownload({
      state: { Failed: 'network unavailable' },
      id: 9,
      name: 'weights.bin',
      completed: 0,
      total: 0,
    })
    expect(useKoharuStore.getState().downloads[9]).toMatchObject({
      state: { Failed: 'network unavailable' },
      name: 'weights.bin',
    })

    useKoharuStore.getState().dismissDownload(9)
    expect(useKoharuStore.getState().downloads[9]).toBeUndefined()
  })

  it('defaults paint brush color to pure white (#FFFFFF)', () => {
    expect(useKoharuStore.getState().brush.color).toBe('#FFFFFF')
  })
})
