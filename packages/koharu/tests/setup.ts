import '@testing-library/jest-dom/vitest'
import { afterEach, beforeEach, vi } from 'vitest'

import { queryClient } from '@/lib/queries'
import { defaultShortcuts, useKoharuStore } from '@/lib/store'

vi.mock('react-i18next', async (importOriginal) => {
  const actual = await importOriginal<typeof import('react-i18next')>()
  const { createInstance } = await import('i18next')
  const { default: translations } = await import('@/public/locales/en-US/translation.json')
  const i18n = createInstance()
  await i18n.init({
    resources: { 'en-US': { translation: translations } },
    lng: 'en-US',
    fallbackLng: 'en-US',
    interpolation: { escapeValue: false },
  })

  return {
    ...actual,
    useTranslation: () => ({ t: i18n.t.bind(i18n), i18n }),
    initReactI18next: { type: '3rdParty', init: () => undefined },
    I18nextProvider: ({ children }: { children: React.ReactNode }) => children,
  }
})

vi.mock('@tauri-apps/api/window', () => {
  const window = {
    close: vi.fn(async () => undefined),
    isMaximized: vi.fn(async () => false),
    minimize: vi.fn(async () => undefined),
    onResized: vi.fn(async () => () => undefined),
    startResizeDragging: vi.fn(async () => undefined),
    toggleMaximize: vi.fn(async () => undefined),
  }
  return { getCurrentWindow: () => window }
})

class Observer {
  observe() {}
  unobserve() {}
  disconnect() {}
}

Object.defineProperty(globalThis, 'ResizeObserver', { value: Observer, writable: true })
Object.defineProperty(window, 'matchMedia', {
  writable: true,
  value: vi.fn((query: string) => ({
    matches: false,
    media: query,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
  })),
})
Element.prototype.scrollIntoView = vi.fn()
Element.prototype.setPointerCapture = vi.fn()
Element.prototype.hasPointerCapture = vi.fn(() => true)
Element.prototype.releasePointerCapture = vi.fn()
Element.prototype.getAnimations = vi.fn(() => [])
Object.defineProperty(URL, 'createObjectURL', {
  value: vi.fn(() => 'blob:koharu-thumbnail'),
  writable: true,
})
Object.defineProperty(URL, 'revokeObjectURL', { value: vi.fn(), writable: true })

const initial = useKoharuStore.getState()
beforeEach(() => {
  queryClient.clear()
  useKoharuStore.setState({
    ...initial,
    initialized: false,
    preferences: null,
    translationModels: [],
    resources: null,
    jobs: {},
    downloads: {},
    camera: { zoom: 1, translation: [0, 0], fitted: true },
    canvasPage: null,
    canvasRevision: null,
    canvasGeneration: 0,
    canvasSize: [0, 0],
    fitCanvasRequest: 0,
    layerFrames: {},
    selectedLayers: [],
    selectedPages: [],
    tool: 'select',
    brush: { diameter: 48, color: '#FFFFFF' },
    inspector: 'copy',
    processingScope: 'page',
    processingStages: ['detection', 'ocr', 'translation', 'inpainting'],
    settingsOpen: false,
    shortcuts: defaultShortcuts,
  })
})

afterEach(() => {
  vi.restoreAllMocks()
})
