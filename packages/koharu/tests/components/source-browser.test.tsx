import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { StrictMode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { SourceBrowser } from '@/components/app/SourceBrowser'
import { queryClient } from '@/lib/queries'
import { useKoharuStore } from '@/lib/store'
import { commands } from '@koharu/bridge/protocol'

vi.mock('@tauri-apps/api/core', async (original) => ({
  ...(await original<typeof import('@tauri-apps/api/core')>()),
  Channel: class {
    onmessage = () => {}
  },
}))

const source = {
  url: 'https://rawkuma.net/manga/example/chapter-16/',
  title: 'Chapter 16',
  images: ['https://cdn.rawkuma.com/s/example/16/1.jpg'],
}

beforeEach(() => {
  vi.spyOn(commands, 'openSourceBrowser').mockResolvedValue(null)
  vi.spyOn(commands, 'closeSourceBrowser').mockResolvedValue(null)
  vi.spyOn(commands, 'inspectSourcePage').mockResolvedValue(source)
  vi.spyOn(commands, 'process').mockResolvedValue('translation-job')
  vi.spyOn(queryClient, 'invalidateQueries').mockResolvedValue(undefined)
})

afterEach(cleanup)

describe('source chapter translation', () => {
  it('opens the saved chapter and does not offer the previous retained chapter for translation', async () => {
    const target = 'https://rawkuma.net/manga/example/chapter-38-5/'
    useKoharuStore.getState().openProjectSource(target)
    vi.spyOn(commands, 'inspectSourcePage').mockResolvedValue(source)
    render(<SourceBrowser />)
    await waitFor(() =>
      expect(commands.openSourceBrowser).toHaveBeenCalledWith(expect.any(Object), target),
    )
    expect(screen.getByRole('textbox', { name: 'Rawkuma address' })).toHaveValue(target)
    expect(screen.getByRole('button', { name: 'Translate this chapter' })).toBeDisabled()
    expect(commands.process).not.toHaveBeenCalled()
  })

  it('does not hide the second browser surface after StrictMode replays the first mount', async () => {
    render(
      <StrictMode>
        <SourceBrowser />
      </StrictMode>,
    )
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Translate this chapter' })).toBeEnabled(),
    )
    expect(commands.openSourceBrowser).toHaveBeenCalledTimes(2)
    expect(commands.closeSourceBrowser).toHaveBeenCalledTimes(1)
  })
  it('opens a completed cached project without another model run', async () => {
    vi.spyOn(commands, 'prepareSourceTranslation').mockResolvedValue({
      project: 'Saved chapter',
      cached: true,
      pages: 1,
    })
    render(<SourceBrowser />)
    const translate = await screen.findByRole('button', { name: 'Translate this chapter' })
    await waitFor(() => expect(translate).toBeEnabled())
    fireEvent.click(translate)
    await waitFor(() => expect(commands.prepareSourceTranslation).toHaveBeenCalled())
    await waitFor(() => expect(useKoharuStore.getState().sourceBusy).toBe(false))
    expect(commands.process).not.toHaveBeenCalled()
  })

  it('starts the existing full pipeline for a new source project', async () => {
    vi.spyOn(commands, 'prepareSourceTranslation').mockResolvedValue({
      project: 'New chapter',
      cached: false,
      pages: 1,
    })
    render(<SourceBrowser />)
    const translate = screen.getByRole('button', { name: 'Translate this chapter' })
    await waitFor(() => expect(translate).toBeEnabled())
    fireEvent.click(translate)
    await waitFor(() =>
      expect(commands.process).toHaveBeenCalledWith({ scope: 'project' }, { operation: 'full' }),
    )
  })

  it('keeps the source view available when importing fails and does not start inference', async () => {
    vi.spyOn(commands, 'prepareSourceTranslation').mockRejectedValue(
      new Error('image download failed'),
    )
    render(<SourceBrowser />)
    const translate = screen.getByRole('button', { name: 'Translate this chapter' })
    await waitFor(() => expect(translate).toBeEnabled())
    fireEvent.click(translate)
    await screen.findByText('image download failed')
    expect(commands.process).not.toHaveBeenCalled()
    expect(useKoharuStore.getState().sourceBusy).toBe(false)
  })
})
