'use client'

import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
} from 'react'

import { useKoharuStore, type CanvasTool } from '@/lib/store'

type ApplyColor = (color: string) => void

interface ColorSampling {
  request: (apply: ApplyColor) => void
  complete: (color: string) => boolean
  cancel: () => void
}

const ColorSamplingContext = createContext<ColorSampling | null>(null)

export function ColorSamplingProvider({ children }: { children: ReactNode }) {
  const pending = useRef<ApplyColor | null>(null)
  const previousTool = useRef<CanvasTool>('select')
  const tool = useKoharuStore((state) => state.tool)
  const setTool = useKoharuStore((state) => state.setTool)

  const request = useCallback(
    (apply: ApplyColor) => {
      if (!pending.current) previousTool.current = useKoharuStore.getState().tool
      pending.current = apply
      setTool('color_picker')
    },
    [setTool],
  )

  const cancel = useCallback(() => {
    const active = pending.current !== null
    pending.current = null
    if (active && useKoharuStore.getState().tool === 'color_picker') {
      setTool(previousTool.current)
    }
  }, [setTool])

  const complete = useCallback(
    (color: string) => {
      const apply = pending.current
      if (!apply) return false
      pending.current = null
      apply(color)
      if (useKoharuStore.getState().tool === 'color_picker') setTool(previousTool.current)
      return true
    },
    [setTool],
  )

  useEffect(() => {
    if (tool !== 'color_picker') cancel()
  }, [cancel, tool])

  useEffect(
    () =>
      useKoharuStore.subscribe((state, previous) => {
        if (!state.picked || state.picked === previous.picked) return
        if (!complete(state.picked.color)) state.setBrush({ ...state.brush, color: state.picked.color })
      }),
    [complete],
  )

  const value = useMemo(() => ({ request, complete, cancel }), [cancel, complete, request])
  return <ColorSamplingContext.Provider value={value}>{children}</ColorSamplingContext.Provider>
}

export function useColorSampling(): ColorSampling | null {
  return useContext(ColorSamplingContext)
}
