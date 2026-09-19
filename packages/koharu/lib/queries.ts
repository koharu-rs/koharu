'use client'

import {
  QueryClient,
  type QueryKey,
  type UseMutationResult,
  queryOptions,
  useIsMutating,
  useMutation,
  useQuery,
} from '@tanstack/react-query'
import { useEffect, useRef } from 'react'

import { commands } from '@koharu/bridge'
import type {
  FontFamily,
  GlossaryEntryDraft,
  GlossaryEntryPatch,
  GlossaryImportPreview,
  GlossaryImportStrategy,
  GlossaryView,
} from '@koharu/bridge/protocol'

import { call } from './backend'
import { createGlossaryEntryUpdateQueue, type GlossaryEntryUpdateQueue } from './glossary'

export const projectKey = ['project'] as const
export const pagesKey = ['pages'] as const
export const pageKey = ['page'] as const
export const glossaryKey = ['glossary'] as const
export const preparedPageKey = (page: string) => ['prepared-page', page] as const
export const fontsKey = ['fonts'] as const

const projectQuery = queryOptions({
  queryKey: projectKey,
  queryFn: () => call(commands.getProject),
})

const pagesQuery = queryOptions({
  queryKey: pagesKey,
  queryFn: () => call(commands.getPages),
})

const pageQuery = queryOptions({
  queryKey: pageKey,
  queryFn: () => call(commands.getPage),
})

const glossaryQuery = queryOptions({
  queryKey: glossaryKey,
  queryFn: () => call(commands.getGlossary),
})

const fontsQuery = queryOptions({
  queryKey: fontsKey,
  queryFn: () => call(commands.getFonts),
})

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: Number.POSITIVE_INFINITY,
      retry: false,
      refetchOnReconnect: false,
      refetchOnWindowFocus: false,
    },
  },
})

export function useProject(enabled = true) {
  return useQuery({ ...projectQuery, enabled })
}

export function usePages(enabled = true) {
  return useQuery({ ...pagesQuery, enabled })
}

export function usePage(enabled = true) {
  return useQuery({ ...pageQuery, enabled })
}

export function useGlossary(enabled = true) {
  return useQuery({ ...glossaryQuery, enabled })
}

export function useFonts(enabled = true) {
  return useQuery({ ...fontsQuery, enabled })
}

export function useFontPreview(font: FontFamily | undefined, enabled = true) {
  return useQuery({
    queryKey: ['font-preview', font?.name],
    queryFn: async () => {
      if (!font) return null
      try {
        return new Uint8Array(await commands.getFontPreview(font.name))
      } catch {
        return null
      }
    },
    enabled: enabled && font !== undefined,
    gcTime: 5 * 60 * 1000,
  })
}

export function useCommand<Args extends unknown[], Result>(
  key: QueryKey,
  command: (...args: Args) => Promise<Result>,
  label: string,
  onSuccess?: () => Promise<void>,
) {
  const busy = useIsMutating({ mutationKey: key }) > 0
  const mutation = useMutation({
    mutationKey: key,
    mutationFn: (args: Args) => call(command, ...args),
    meta: { activity: label },
    onSuccess,
  })
  return { run: (...args: Args) => mutation.mutate(args), busy }
}

export function useImportPages() {
  const { run, busy } = useCommand(['import-pages'], commands.import, 'navigator.importing', () =>
    refresh(projectKey, pagesKey, pageKey),
  )
  return { importPages: run, importing: busy }
}

async function updateGlossaryCaches(view?: GlossaryView): Promise<void> {
  if (view) queryClient.setQueryData(glossaryKey, view)
  await refresh(projectKey, glossaryKey)
}

async function recoverGlossaryMutation(): Promise<void> {
  await refresh(projectKey, glossaryKey)
}

export function useScanGlossary() {
  return useMutation({
    mutationKey: ['glossary', 'scan'],
    mutationFn: () => call(commands.scanGlossary),
    meta: { activity: 'glossary.scanning' },
    onSuccess: () => updateGlossaryCaches(),
    onError: recoverGlossaryMutation,
  })
}

export function useSetGlossaryEnabled() {
  return useMutation({
    mutationKey: ['glossary', 'toggle'],
    mutationFn: ({ revision, enabled }: { revision: number; enabled: boolean }) =>
      call(commands.setGlossaryEnabled, revision, enabled),
    onSuccess: updateGlossaryCaches,
    onError: recoverGlossaryMutation,
  })
}

export function useAddGlossaryEntry() {
  return useMutation({
    mutationKey: ['glossary', 'add'],
    mutationFn: ({ revision, draft }: { revision: number; draft: GlossaryEntryDraft }) =>
      call(commands.addGlossaryEntry, revision, draft),
    onSuccess: updateGlossaryCaches,
    onError: recoverGlossaryMutation,
  })
}

export function useUpdateGlossaryEntry(revision: number, scope = '') {
  const owner = useRef<{ scope: string; queue: GlossaryEntryUpdateQueue } | null>(null)
  if (owner.current === null || owner.current.scope !== scope) {
    owner.current = {
      scope,
      queue: createGlossaryEntryUpdateQueue({
        initialRevision: revision,
        execute: (expectedRevision, id, patch) =>
          call(commands.updateGlossaryEntry, expectedRevision, id, patch),
        onResponse: async (view, current) => {
          if (current) queryClient.setQueryData(glossaryKey, view)
          await Promise.all([
            queryClient.invalidateQueries({ queryKey: projectKey }),
            queryClient.invalidateQueries({
              queryKey: glossaryKey,
              refetchType: current ? 'active' : 'none',
            }),
          ])
        },
      }),
    }
  }
  useEffect(() => owner.current?.queue.setRevision(revision), [revision])

  return useMutation({
    mutationKey: ['glossary', 'update'],
    mutationFn: ({ id, patch }: { id: string; patch: GlossaryEntryPatch }) =>
      owner.current!.queue.enqueue(id, patch),
    onError: recoverGlossaryMutation,
  })
}

export function useDeleteGlossaryEntries() {
  return useMutation({
    mutationKey: ['glossary', 'delete'],
    mutationFn: ({ revision, ids }: { revision: number; ids: string[] }) =>
      call(commands.deleteGlossaryEntries, revision, ids),
    onSuccess: updateGlossaryCaches,
    onError: recoverGlossaryMutation,
  })
}

export function useTranslateGlossaryEntries() {
  return useMutation({
    mutationKey: ['glossary', 'translate'],
    mutationFn: ({ revision, ids }: { revision: number; ids: string[] | null }) =>
      call(commands.translateGlossaryEntries, revision, ids),
    meta: { activity: 'glossary.translating' },
    onSuccess: () => updateGlossaryCaches(),
    onError: recoverGlossaryMutation,
  })
}

export function usePreviewGlossaryImport(): UseMutationResult<
  GlossaryImportPreview,
  Error,
  string
> {
  return useMutation({
    mutationKey: ['glossary', 'import-preview'],
    mutationFn: (document: string) => call(commands.previewGlossaryImport, document),
  })
}

export function useApplyGlossaryImport() {
  return useMutation({
    mutationKey: ['glossary', 'import-apply'],
    mutationFn: ({
      revision,
      document,
      strategy,
      confirmLanguageMismatch,
    }: {
      revision: number
      document: string
      strategy: GlossaryImportStrategy
      confirmLanguageMismatch: boolean
    }) => call(commands.applyGlossaryImport, revision, document, strategy, confirmLanguageMismatch),
    onSuccess: updateGlossaryCaches,
    onError: recoverGlossaryMutation,
  })
}

export function useExportGlossary() {
  return useMutation({
    mutationKey: ['glossary', 'export'],
    mutationFn: () => call(commands.exportGlossary),
    meta: { activity: 'glossary.exporting' },
  })
}

export async function refresh(...keys: QueryKey[]): Promise<void> {
  await Promise.all(keys.map((queryKey) => queryClient.invalidateQueries({ queryKey })))
}
