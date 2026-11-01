import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { CanvasCommandBar } from '@/components/editor/CanvasCommandBar'
import { useKoharuStore } from '@/lib/store'
import { commands } from '@koharu/bridge/protocol'

vi.mock('@/lib/queries', async (original) => ({
  ...(await original<typeof import('@/lib/queries')>()),
  usePage: () => ({ data: { id: 'current-page' } }),
}))

beforeEach(() => {
  vi.spyOn(commands, 'process').mockResolvedValue('job')
})
afterEach(cleanup)

describe('project processing controls', () => {
  it('runs the full project even when only one page is selected', () => {
    useKoharuStore.setState({ selectedPages: ['current-page'] })
    render(<CanvasCommandBar />)
    fireEvent.click(screen.getByRole('button', { name: 'Run processing' }))
    expect(commands.process).toHaveBeenCalledWith({ scope: 'project' }, { operation: 'full' })
    fireEvent.click(screen.getByRole('button', { name: 'Retranslate' }))
    expect(commands.process).toHaveBeenLastCalledWith(
      { scope: 'project' },
      { operation: 'only', stage: 'translation' },
    )
  })

  it('retranslates the chosen pages without changing the stages configured for Run', () => {
    useKoharuStore.setState({
      processingScope: 'selected-pages',
      selectedPages: ['page-a', 'page-b'],
      processingStages: ['ocr', 'translation'],
    })
    render(<CanvasCommandBar />)
    fireEvent.click(screen.getByRole('button', { name: 'Retranslate' }))
    expect(commands.process).toHaveBeenCalledWith(
      { scope: 'pages', value: ['page-a', 'page-b'] },
      { operation: 'only', stage: 'translation' },
    )
    fireEvent.click(screen.getByRole('button', { name: 'Run processing' }))
    expect(commands.process).toHaveBeenLastCalledWith(
      { scope: 'pages', value: ['page-a', 'page-b'] },
      { operation: 'stages', stages: ['ocr', 'translation'] },
    )
  })

  it('disables both actions when selected-pages scope has no selection', () => {
    useKoharuStore.setState({ processingScope: 'selected-pages', selectedPages: [] })
    render(<CanvasCommandBar />)
    expect(screen.getByRole('button', { name: 'Run processing' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Retranslate' })).toBeDisabled()
    expect(commands.process).not.toHaveBeenCalled()
  })
})
