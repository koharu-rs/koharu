import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { ExportCbzDialog } from '@/components/app/ExportCbzDialog'
import { ActivityCenter } from '@/components/editor/ActivityCenter'
import { useKoharuStore } from '@/lib/store'
import { commands } from '@koharu/bridge/protocol'

describe('CBZ export', () => {
  it('exports the entire project with a persisted WebP quality', async () => {
    vi.spyOn(commands, 'getExportConfig').mockResolvedValue({ jpeg_quality: 90, webp_quality: 85 })
    const save = vi
      .spyOn(commands, 'saveExportConfig')
      .mockResolvedValue({ jpeg_quality: 90, webp_quality: 85 })
    const start = vi.spyOn(commands, 'exportCbz').mockResolvedValue('export-job')
    const close = vi.fn()
    useKoharuStore.setState({ selectedPages: ['selected-only'] })
    render(<ExportCbzDialog open onOpenChange={close} />)
    expect(screen.getByText(/every page in project order/)).toBeInTheDocument()
    fireEvent.change(screen.getByLabelText('Archive image format'), { target: { value: 'webp' } })
    expect(await screen.findByLabelText('Quality (1–100)')).toHaveValue(85)
    fireEvent.click(screen.getByRole('button', { name: 'Export' }))
    await waitFor(() => expect(start).toHaveBeenCalledWith('webp'))
    expect(save).toHaveBeenCalledWith({ jpeg_quality: 90, webp_quality: 85 })
    expect(close).toHaveBeenCalledWith(false)
  })

  it('shows export progress and cancels through the shared job command', async () => {
    const stop = vi.spyOn(commands, 'stopJob').mockResolvedValue(null)
    useKoharuStore.setState({
      jobs: {
        export: {
          id: 'export',
          kind: 'export',
          workflow: null,
          state: 'running',
          completed: 3,
          total: 20,
          page: 'page',
          stage: null,
          model: null,
          error: null,
        },
      },
    })
    render(<ActivityCenter />)
    expect(screen.getByText('Exporting')).toBeInTheDocument()
    expect(screen.getByText('3 / 20')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }))
    await waitFor(() => expect(stop).toHaveBeenCalledWith('export'))
  })
})
