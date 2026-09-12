import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { WorkflowPreferences } from '@/components/preferences/WorkflowPreferences'
import { commands, type WorkflowSettings } from '@koharu/bridge/protocol'

describe('project workflow settings', () => {
  it('persists the switch and selected preset without replacing the preset list', async () => {
    const settings: WorkflowSettings = {
      enabled: false,
      active_preset: 'Consistent Project Translation',
      presets: [{ name: 'Standard' }, { name: 'Consistent Project Translation' }],
    }
    vi.spyOn(commands, 'getWorkflowSettings').mockResolvedValue(settings)
    const save = vi
      .spyOn(commands, 'configureProjectWorkflow')
      .mockImplementation(async (enabled, active_preset) => ({
        ...settings,
        enabled,
        active_preset,
      }))
    render(<WorkflowPreferences />)
    const toggle = await screen.findByRole('switch', { name: 'Use project workflow for Run All' })
    await waitFor(() => expect(toggle).not.toBeDisabled())
    fireEvent.click(toggle)
    await waitFor(() => expect(save).toHaveBeenCalledWith(true, 'Consistent Project Translation'))
    await waitFor(() => expect(toggle).toBeChecked())
    fireEvent.change(screen.getByLabelText('Preset'), { target: { value: 'Standard' } })
    await waitFor(() => expect(save).toHaveBeenCalledWith(true, 'Standard'))
  })
})
