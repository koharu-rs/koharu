import type { TFunction } from 'i18next'
import { describe, expect, it } from 'vitest'

import {
  classifyVramFit,
  computeOtherStagesVram,
  findCatalogSelection,
  formatDiskSize,
  formatModelName,
  formatVramSize,
} from '@/lib/models'
import type { ModelCatalog, PipelineConfig, TranslateModelEntry } from '@koharu/ipc/protocol'

const GB_DISK = 1_000_000_000
const GIB = 1024 * 1024 * 1024

const t = ((key: string, params?: Record<string, unknown>) => {
  if (key === 'modelPicker.quantizationLabel') {
    return `${params?.model} (${params?.quantization})`
  }
  if (key.startsWith('modelPicker.units.')) {
    const unit = key.split('.').pop()!.toUpperCase()
    return `${params?.value} ${unit}`
  }
  return key
}) as unknown as TFunction

const sampleTranslate: TranslateModelEntry[] = [
  {
    reference: 'koharu/gemma4-e2b-it',
    name: 'Gemma 4 E2B IT',
    vendor: 'koharu',
    images: true,
    tools: true,
    disk_bytes: 3_100_000_000,
    downloaded: true,
    measured_vram_bytes: 4 * GIB,
    variants: [
      {
        id: 'Q4_K_XL',
        reference: 'koharu/gemma4-e2b-it:Q4_K_XL',
        name: 'Q4_K_XL',
        disk_bytes: 3_100_000_000,
        downloaded: true,
        measured_vram_bytes: 4 * GIB,
      },
      {
        id: 'Q2_K_XL',
        reference: 'koharu/gemma4-e2b-it:Q2_K_XL',
        name: 'Q2_K_XL',
        disk_bytes: 2_200_000_000,
        downloaded: false,
        measured_vram_bytes: 2.5 * GIB,
      },
    ],
  },
  {
    reference: 'ollama/llama3:8b',
    name: 'Llama 3 8B',
    vendor: 'ollama',
    images: false,
    tools: true,
    disk_bytes: null,
    downloaded: false,
    measured_vram_bytes: null,
    variants: [],
  },
  {
    reference: 'openrouter/deepseek/r1:free',
    name: 'DeepSeek R1 Free',
    vendor: 'openrouter',
    images: false,
    tools: false,
    disk_bytes: null,
    downloaded: false,
    measured_vram_bytes: null,
    variants: [],
  },
]

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
      measured_vram_bytes: 6 * GIB,
    },
  ],
  translate: sampleTranslate,
}

describe('model catalog helpers', () => {
  it('classifies VRAM fit boundaries and returns null when unmeasured', () => {
    expect(classifyVramFit(null, 0, 8 * GIB)).toBeNull()
    expect(classifyVramFit(4 * GIB, 0, null)).toBeNull()
    expect(classifyVramFit(4 * GIB, 0, 0)).toBeNull()

    expect(classifyVramFit(4 * GIB, 2 * GIB, 8 * GIB)).toBe('fits')
    expect(classifyVramFit(6 * GIB, 2 * GIB, 8 * GIB)).toBe('fits')
    expect(classifyVramFit(6 * GIB, 3 * GIB, 8 * GIB)).toBe('swaps')
    expect(classifyVramFit(9 * GIB, 0, 8 * GIB)).toBe('tooLarge')
  })

  it('resolves local quantizations while preserving colons in hosted references', () => {
    expect(findCatalogSelection(sampleTranslate, 'koharu/gemma4-e2b-it')).toMatchObject({
      explicitQuant: false,
      variant: { id: 'Q4_K_XL' },
    })
    expect(findCatalogSelection(sampleTranslate, 'koharu/gemma4-e2b-it:Q2_K_XL')).toMatchObject({
      explicitQuant: true,
      variant: { id: 'Q2_K_XL' },
    })
    expect(formatModelName(sampleTranslate, 'koharu/gemma4-e2b-it:Q2_K_XL', t)).toBe(
      'Gemma 4 E2B IT (Q2_K_XL)',
    )
    expect(formatModelName(sampleTranslate, 'ollama/llama3:8b', t)).toBe('Llama 3 8B')
    expect(formatModelName(sampleTranslate, 'openrouter/deepseek/r1:free', t)).toBe(
      'DeepSeek R1 Free',
    )
    expect(formatModelName(sampleTranslate, 'unknown/model', t)).toBe('unknown/model')
  })

  it('computes combined VRAM used by other stages', () => {
    const pipeline: PipelineConfig = {
      detector: { model: 'koharu-layout-highres' },
      recognizer: { model: 'hayai-ocr' },
      inpainter: { model: 'flux3-klein' },
      translator: { model: 'koharu/gemma4-e2b-it:Q2_K_XL' },
    }
    expect(computeOtherStagesVram(sampleCatalog, pipeline, 'translate')).toBe(
      512 * 1024 * 1024 + 1024 * 1024 * 1024 + 6 * GIB,
    )
    expect(computeOtherStagesVram(sampleCatalog, pipeline, 'inpaint')).toBe(
      512 * 1024 * 1024 + 1024 * 1024 * 1024 + 2.5 * GIB,
    )
    // Hosted translator contributes 0.
    expect(
      computeOtherStagesVram(
        sampleCatalog,
        { ...pipeline, translator: { model: 'ollama/llama3:8b' } },
        'inpaint',
      ),
    ).toBe(512 * 1024 * 1024 + 1024 * 1024 * 1024)
  })

  it('formats disk bytes (base-1000) and VRAM bytes (base-1024)', () => {
    expect(formatDiskSize(3.1 * GB_DISK, t, 'en-US')).toBe('3.1 GB')
    expect(formatVramSize(8 * GIB, t, 'en-US')).toBe('8.0 GB')
  })
})
