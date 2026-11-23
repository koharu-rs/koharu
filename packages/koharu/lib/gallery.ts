'use client'

import { create } from 'zustand'
import { persist } from 'zustand/middleware'

import type { Reading, SeriesLink } from '@koharu/ipc/protocol'

/** How a chapter's pages read, as its site said when it was read into its project. */
export type ChapterReading = Omit<Reading, 'project'>

interface GalleryState {
  /** The series the user opened, the last first. */
  shelf: SeriesLink[]
  /** How each chapter read into a project reads, by the project's name. */
  readings: Record<string, ChapterReading>
  /** Puts `series` first on the shelf. */
  keep: (series: SeriesLink) => void
  forget: (url: string) => void
  /** Keeps how the chapter a project was read from reads. */
  remember: (reading: Reading) => void
}

/** What the gallery keeps between runs, in this browser's storage: nothing the desktop needs. */
export const useGallery = create<GalleryState>()(
  persist(
    (set) => ({
      shelf: [],
      readings: {},
      keep: (series) =>
        set((state) => ({
          shelf: [series, ...state.shelf.filter((kept) => kept.url !== series.url)],
        })),
      forget: (url) => set((state) => ({ shelf: state.shelf.filter((kept) => kept.url !== url) })),
      remember: ({ project, ...reading }) =>
        set((state) => ({ readings: { ...state.readings, [project]: reading } })),
    }),
    {
      name: 'koharu.gallery',
      partialize: ({ shelf, readings }) => ({ shelf, readings }),
    },
  ),
)
