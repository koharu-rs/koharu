import { QueryClientProvider } from '@tanstack/react-query'
import { render as testingRender, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { ReactNode } from 'react'
import { describe, expect, it, vi } from 'vitest'

import { ModelPicker } from '@/components/controls/ModelPicker'
import { AgentPanel } from '@/components/editor/AgentPanel'
import { PipelinePreferences } from '@/components/preferences/PipelinePreferences'
import { queryClient } from '@/lib/queries'
import {
  commands,
  type ModelCatalog,
  type PipelineConfig,
  type Preferences,
} from '@koharu/ipc/protocol'
import { TooltipProvider } from '@koharu/ui/components/tooltip'

vi.mock('@tauri-apps/api/core', () => ({
  Channel: class {},
  invoke: vi.fn(),
}))

function render(ui: ReactNode) {
  return testingRender(
    <QueryClientProvider client={queryClient}>
      <TooltipProvider delay={0}>{ui}</TooltipProvider>
    </QueryClientProvider>,
  )
}

const GIB = 1024 * 1024 * 1024

const sampleCatalog: ModelCatalog = {
  available_vram_bytes: 8 * GIB,
  total_vram_bytes: 12 * GIB,
  detect: [
    {
      id: 'koharu-layout-highres',
      name: 'Koharu Layout HighRes',
      stage: 'detect',
      disk_bytes: 200_000_000,
      downloaded: true,
      measured_vram_bytes: 512 * 1024 * 1024,
    },
  ],
  recognize: [
    {
      id: 'hayai-ocr',
      name: 'Hayai OCR',
      stage: 'recognize',
      disk_bytes: 300_000_000,
      downloaded: true,
      measured_vram_bytes: 1024 * 1024 * 1024,
    },
  ],
  inpaint: [
    {
      id: 'lama',
      name: 'LaMa',
      stage: 'inpaint',
      disk_bytes: 200_000_000,
      downloaded: true,
      measured_vram_bytes: 768 * 1024 * 1024,
    },
    {
      id: 'flux3-klein',
      name: 'FLUX.3 Klein',
      stage: 'inpaint',
      disk_bytes: 7_000_000_000,
      downloaded: false,
      measured_vram_bytes: 9 * GIB,
    },
  ],
  translate: [
    {
      reference: 'koharu/gemma4-e2b-it',
      name: 'Gemma 4 E2B IT',
      vendor: 'koharu',
      images: true,
      tools: true,
      disk_bytes: 3_100_000_000,
      downloaded: true,
      measured_vram_bytes: 6 * GIB,
      variants: [
        {
          id: 'Q4_K_XL',
          reference: 'koharu/gemma4-e2b-it:Q4_K_XL',
          name: 'Q4_K_XL',
          disk_bytes: 3_100_000_000,
          downloaded: true,
          measured_vram_bytes: 6 * GIB,
        },
        {
          id: 'Q2_K_XL',
          reference: 'koharu/gemma4-e2b-it:Q2_K_XL',
          name: 'Q2_K_XL',
          disk_bytes: 2_200_000_000,
          downloaded: false,
          measured_vram_bytes: 2 * GIB,
        },
      ],
    },
    {
      reference: 'koharu/lfm3-1.7b',
      name: 'LFM3 1.7B',
      vendor: 'koharu',
      images: false,
      tools: false,
      disk_bytes: 1_200_000_000,
      downloaded: false,
      measured_vram_bytes: null,
      variants: [
        {
          id: 'Q4_K_M',
          reference: 'koharu/lfm3-1.7b:Q4_K_M',
          name: 'Q4_K_M',
          disk_bytes: 1_200_000_000,
          downloaded: false,
          measured_vram_bytes: null,
        },
      ],
    },
    {
      reference: 'openai/gpt-5',
      name: 'GPT-5',
      vendor: 'openai',
      images: true,
      tools: true,
      disk_bytes: null,
      downloaded: false,
      measured_vram_bytes: null,
      variants: [],
    },
  ],
}

describe('ModelPicker & stage catalog UI', () => {
  it('renders 1 row per family, suppresses traffic lights when unmeasured, and drills into quantizations', async () => {
    const user = userEvent.setup()
    const onSelect = vi.fn()

    render(
      <ModelPicker
        value='koharu/gemma4-e2b-it'
        models={sampleCatalog.translate}
        availableVramBytes={8 * GIB}
        otherStagesVramBytes={3 * GIB}
        onSelect={onSelect}
      />,
    )

    // Family rows render (quantizations are not flattened at the root level).
    expect(await screen.findByText('Gemma 4 E2B IT')).toBeInTheDocument()
    expect(screen.getByText('LFM3 1.7B')).toBeInTheDocument()
    expect(screen.getByText('GPT-5')).toBeInTheDocument()
    expect(screen.queryByText('Q2_K_XL')).not.toBeInTheDocument()

    // Measured 6 GiB + 3 GiB other stages > 8 GiB free -> Swaps badge with exact tooltip.
    const swapsBadge = screen.getByText('Swaps')
    expect(swapsBadge).toBeInTheDocument()
    await user.hover(swapsBadge)
    expect(
      await screen.findByText('≈6.0 GB (+ 3.0 GB other stages) of 8.0 GB free'),
    ).toBeInTheDocument()

    // Unmeasured local model (LFM3 1.7B) and hosted model (GPT-5) render NO traffic light.
    expect(screen.queryByText('Too large')).not.toBeInTheDocument()
    expect(screen.queryByText('Fits')).not.toBeInTheDocument()

    // 1-variant family has no quantization chevron; multi-variant family has one.
    expect(
      screen.queryByRole('button', { name: 'Choose quantization for LFM3 1.7B' }),
    ).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Choose quantization for Gemma 4 E2B IT' }))

    // Submenu lists both quantizations; Q2_K_XL (2 GiB + 3 GiB <= 8 GiB) shows Fits.
    expect(await screen.findByText('Q2_K_XL')).toBeInTheDocument()
    expect(screen.getByText('Fits')).toBeInTheDocument()

    await user.click(screen.getByRole('option', { name: 'Q2_K_XL' }))
    expect(onSelect).toHaveBeenCalledWith('koharu/gemma4-e2b-it:Q2_K_XL')
  })

  it('hides all VRAM fit badges when available VRAM telemetry is null', async () => {
    render(
      <ModelPicker
        value='koharu/gemma4-e2b-it'
        models={sampleCatalog.translate}
        availableVramBytes={null}
        otherStagesVramBytes={0}
        onSelect={vi.fn()}
      />,
    )

    expect(await screen.findByText('Gemma 4 E2B IT')).toBeInTheDocument()
    expect(screen.queryByText('Fits')).not.toBeInTheDocument()
    expect(screen.queryByText('Swaps')).not.toBeInTheDocument()
    expect(screen.queryByText('Too large')).not.toBeInTheDocument()
  })

  it('renders all 4 stages in PipelinePreferences with disk sizes and VRAM fit badges', async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    const onRefreshCatalog = vi.fn()
    const pipeline: PipelineConfig = {
      detector: { model: 'koharu-layout-highres' },
      recognizer: { model: 'hayai-ocr' },
      inpainter: { model: 'lama' },
      translator: { model: 'koharu/gemma4-e2b-it:Q2_K_XL' },
    }

    render(
      <PipelinePreferences
        value={pipeline}
        catalog={sampleCatalog}
        onRefreshCatalog={onRefreshCatalog}
        onChange={onChange}
      />,
    )

    const inpaintTrigger = screen.getByRole('button', { name: 'Inpainter model' })
    expect(inpaintTrigger).toHaveTextContent('LaMa')
    expect(inpaintTrigger).toHaveTextContent('200.0 MB')
    expect(inpaintTrigger).toHaveTextContent('Fits')

    await user.click(inpaintTrigger)
    expect(onRefreshCatalog).toHaveBeenCalledTimes(1)

    const fluxOption = await screen.findByRole('button', { name: /FLUX\.3 Klein/ })
    expect(fluxOption).toHaveTextContent('7.0 GB')
    expect(fluxOption).toHaveTextContent('Too large')

    await user.click(fluxOption)
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({
        inpainter: expect.objectContaining({ model: 'flux3-klein' }),
      }),
    )
  })

  it('filters AgentPanel models by tools and refetches catalog on popover open', async () => {
    const user = userEvent.setup()
    const preferences: Preferences = {
      pipeline: {},
      renderer: { styles: { antique: { Latn: { family: 'CCWildWords' } } } },
      agent: { model: 'koharu/gemma4-e2b-it' },
      providers: {},
      export: { cbz_codec: 'png', quality: 90, lossless: false },
    }
    vi.spyOn(commands, 'getPreferences').mockResolvedValue(preferences)
    const catalogSpy = vi.spyOn(commands, 'listModelCatalog').mockResolvedValue(sampleCatalog)

    render(<AgentPanel />)

    const trigger = await screen.findByRole('button', { name: 'Model' })
    await waitFor(() => expect(catalogSpy).toHaveBeenCalledTimes(1))

    await user.click(trigger)
    await waitFor(() => expect(catalogSpy).toHaveBeenCalledTimes(2))

    // Tool-capable models appear; non-tool model (LFM3 1.7B) is filtered out.
    expect(await screen.findByText('GPT-5')).toBeInTheDocument()
    expect(screen.getAllByText('Gemma 4 E2B IT')).toHaveLength(2)
    expect(screen.queryByText('LFM3 1.7B')).not.toBeInTheDocument()
  })

  it('keeps keyboard focus inside VariantSubmenu on ArrowRight, restores search focus on ArrowLeft, and selects variants via ArrowDown + Enter', async () => {
    const user = userEvent.setup()
    const onSelect = vi.fn()

    render(
      <ModelPicker
        value={null}
        models={sampleCatalog.translate}
        availableVramBytes={8 * GIB}
        otherStagesVramBytes={0}
        onSelect={onSelect}
      />,
    )

    const search = await screen.findByRole('combobox', { name: 'Search models' })
    expect(document.activeElement).toBe(search)

    // ArrowDown highlights the first family (Gemma 4 E2B IT, 2 variants), ArrowRight opens submenu.
    await user.keyboard('{ArrowDown}{ArrowRight}')
    const submenuList = await screen.findByRole('listbox', {
      name: 'Choose quantization for Gemma 4 E2B IT',
    })
    expect(document.activeElement).not.toBe(document.body)
    expect(submenuList.contains(document.activeElement)).toBe(true)

    // ArrowLeft returns to root ModelPicker and restores focus to PickerSearch.
    await user.keyboard('{ArrowLeft}')
    const restoredSearch = await screen.findByRole('combobox', { name: 'Search models' })
    expect(document.activeElement).toBe(restoredSearch)
    expect(
      screen.queryByRole('listbox', { name: 'Choose quantization for Gemma 4 E2B IT' }),
    ).not.toBeInTheDocument()

    // Re-enter submenu with ArrowDown -> ArrowRight, then ArrowDown -> Enter selects Q4_K_XL.
    await user.keyboard('{ArrowDown}{ArrowRight}')
    await screen.findByRole('listbox', { name: 'Choose quantization for Gemma 4 E2B IT' })
    await user.keyboard('{ArrowDown}{Enter}')
    expect(onSelect).toHaveBeenCalledWith('koharu/gemma4-e2b-it:Q4_K_XL')
  })
})
