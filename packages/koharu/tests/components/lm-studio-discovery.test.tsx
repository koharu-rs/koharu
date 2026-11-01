import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { LmStudioDiscovery } from '@/components/preferences/LmStudioDiscovery'
import { commands } from '@koharu/bridge/protocol'

beforeEach(() => {
  vi.spyOn(commands, 'getTranslationModels').mockResolvedValue([])
})
afterEach(cleanup)

describe('LM Studio discovery', () => {
  it('uses the configured port and changes the URL only after selecting a discovered server', async () => {
    vi.spyOn(commands, 'discoverLmStudio').mockResolvedValue({
      subnets: ['192.168.31.0/24'],
      servers: [{ base_url: 'http://192.168.31.73:1235', models: ['qwen'] }],
    })
    const select = vi.fn()
    render(<LmStudioDiscovery baseUrl='http://192.168.31.55:1235' onSelect={select} />)
    fireEvent.click(screen.getByRole('button', { name: 'Refresh / find server' }))
    const use = await screen.findByRole('button', { name: 'Use http://192.168.31.73:1235' })
    expect(commands.discoverLmStudio).toHaveBeenCalledWith(1235)
    expect(select).not.toHaveBeenCalled()
    fireEvent.click(use)
    expect(select).toHaveBeenCalledWith('http://192.168.31.73:1235')
  })

  it('preserves the configured URL when a scan finds no server', async () => {
    vi.spyOn(commands, 'discoverLmStudio').mockResolvedValue({
      subnets: ['192.168.31.0/24'],
      servers: [],
    })
    const select = vi.fn()
    render(<LmStudioDiscovery baseUrl='http://192.168.31.73:1234' onSelect={select} />)
    fireEvent.click(screen.getByRole('button', { name: 'Refresh / find server' }))
    await screen.findByText(/No server found on 192.168.31.0\/24/)
    expect(select).not.toHaveBeenCalled()
  })

  it('allows another scan after a native failure and shows the error inline', async () => {
    vi.spyOn(commands, 'discoverLmStudio').mockRejectedValue(new Error('network unavailable'))
    render(<LmStudioDiscovery baseUrl='http://localhost:1234' onSelect={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: 'Refresh / find server' }))
    await screen.findByText('network unavailable')
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Refresh / find server' })).toBeEnabled(),
    )
  })
})
