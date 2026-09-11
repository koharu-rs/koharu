import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { ModelPicker } from '@/components/controls/ModelPicker'
import type { Model, ModelSelection, ProviderPreference } from '@koharu/bridge/protocol'

const providers: ProviderPreference[] = [
  { name: 'Local', config: { provider: 'local', settings: {} }, credential: null },
]

const localModel: Model = {
  provider: 'local',
  model: 'gemma4-e2b-it',
  name: 'Gemma 4 E2B Instruct',
  quantizations: [
    { id: 'Q4_K_M', name: 'Q4_K_M', downloaded: false },
    { id: 'Q8_0', name: 'Q8_0', downloaded: true },
  ],
  vision: true,
  reasoning: true,
}

const selection = (quantization: string): ModelSelection => ({
  provider: 'local',
  model: 'gemma4-e2b-it',
  quantization,
  vision: true,
  reasoning: true,
})

describe('ModelPicker download status', () => {
  it('shows an icon when the effective quantization is downloaded', () => {
    render(
      <ModelPicker
        value={selection('Q8_0')}
        models={[localModel]}
        providers={providers}
        onBack={vi.fn()}
        onSelect={vi.fn()}
      />,
    )
    expect(screen.getByLabelText('Downloaded')).toBeInTheDocument()
    expect(screen.queryByText('Downloaded')).not.toBeInTheDocument()
  })

  it('hides the icon when the effective quantization is not downloaded', () => {
    render(
      <ModelPicker
        value={selection('Q4_K_M')}
        models={[localModel]}
        providers={providers}
        onBack={vi.fn()}
        onSelect={vi.fn()}
      />,
    )
    expect(screen.queryByLabelText('Downloaded')).not.toBeInTheDocument()
  })
})
