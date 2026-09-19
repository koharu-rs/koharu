'use client'

import {
  QueryClient,
  type QueryKey,
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
  GlossaryImportStrategy,
  GlossaryView,
  ProjectInfo,
} from '@koharu/bridge/protocol'

import { call, reportError } from './backend'
import { createGlossaryEntryUpdateQueue, type GlossaryEntryUpdateQueue } from './glossary'

export const projectKey = ['project'] as const
export const pagesKey = ['pages'] as const
export const pageKey = ['page'] as const
export const glossaryKey = (projectName: string | undefined) => ['glossary', projectName] as const
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

export function useGlossary(projectName: string | undefined) {
  return useQuery({
    queryKey: glossaryKey(projectName),
    queryFn: () => commands.getGlossary(),
    enabled: projectName !== undefined,
  })
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

interface GlossaryScope {
  projectName: string | undefined
  generation: number
}

function useGlossaryScope(projectName: string | undefined) {
  const scope = useRef<GlossaryScope>({ projectName, generation: 0 })
  if (scope.current.projectName !== projectName) {
    scope.current = { projectName, generation: scope.current.generation + 1 }
  }
  return scope
}

function captureGlossaryScope(scope: { current: GlossaryScope }): GlossaryScope {
  return { ...scope.current }
}

function currentProjectName(): string | undefined {
  return queryClient.getQueryData<ProjectInfo | null>(projectKey)?.name
}

function glossaryScopeIsCurrent(
  scope: { current: GlossaryScope },
  captured: GlossaryScope | undefined,
): captured is GlossaryScope & { projectName: string } {
  return (
    captured !== undefined &&
    captured.projectName !== undefined &&
    captured.projectName === scope.current.projectName &&
    captured.generation === scope.current.generation &&
    captured.projectName === currentProjectName()
  )
}

async function updateGlossaryCaches(
  scope: { current: GlossaryScope },
  captured: GlossaryScope | undefined,
  view?: GlossaryView,
): Promise<void> {
  if (!glossaryScopeIsCurrent(scope, captured)) return
  const key = glossaryKey(captured.projectName)
  if (view) {
    const project = queryClient.getQueryData<ProjectInfo | null>(projectKey)
    if (!project || view.revision >= project.revision) queryClient.setQueryData(key, view)
  }
  await Promise.all([
    queryClient.invalidateQueries({ queryKey: projectKey, exact: true }),
    queryClient.invalidateQueries({ queryKey: key, exact: true }),
  ])
}

async function recoverGlossaryMutation(
  scope: { current: GlossaryScope },
  captured: GlossaryScope | undefined,
  error: unknown,
): Promise<void> {
  if (!glossaryScopeIsCurrent(scope, captured)) return
  reportError(error)
  await Promise.all([
    queryClient.invalidateQueries({ queryKey: projectKey, exact: true }),
    queryClient.invalidateQueries({ queryKey: glossaryKey(captured.projectName), exact: true }),
  ])
}

export function useScanGlossary(projectName: string | undefined) {
  const scope = useGlossaryScope(projectName)
  return useMutation({
    mutationKey: ['glossary', projectName, 'scan'],
    mutationFn: () => commands.scanGlossary(),
    meta: { activity: 'glossary.scanning' },
    onMutate: () => captureGlossaryScope(scope),
    onSuccess: (_, __, captured) => updateGlossaryCaches(scope, captured),
    onError: (error, _, captured) => recoverGlossaryMutation(scope, captured, error),
  })
}

export function useSetGlossaryEnabled(projectName: string | undefined) {
  const scope = useGlossaryScope(projectName)
  return useMutation({
    mutationKey: ['glossary', projectName, 'toggle'],
    mutationFn: ({ revision, enabled }: { revision: number; enabled: boolean }) =>
      commands.setGlossaryEnabled(revision, enabled),
    onMutate: () => captureGlossaryScope(scope),
    onSuccess: (view, _, captured) => updateGlossaryCaches(scope, captured, view),
    onError: (error, _, captured) => recoverGlossaryMutation(scope, captured, error),
  })
}

export function useAddGlossaryEntry(projectName: string | undefined) {
  const scope = useGlossaryScope(projectName)
  return useMutation({
    mutationKey: ['glossary', projectName, 'add'],
    mutationFn: ({ revision, draft }: { revision: number; draft: GlossaryEntryDraft }) =>
      commands.addGlossaryEntry(revision, draft),
    onMutate: () => captureGlossaryScope(scope),
    onSuccess: (view, _, captured) => updateGlossaryCaches(scope, captured, view),
    onError: (error, _, captured) => recoverGlossaryMutation(scope, captured, error),
  })
}

export function useUpdateGlossaryEntry(revision: number, projectName: string | undefined) {
  const scope = useGlossaryScope(projectName)
  const owner = useRef<{
    scope: GlossaryScope
    queue: GlossaryEntryUpdateQueue
  } | null>(null)
  if (
    owner.current === null ||
    owner.current.scope.projectName !== scope.current.projectName ||
    owner.current.scope.generation !== scope.current.generation
  ) {
    const ownerScope = captureGlossaryScope(scope)
    let nextOwner!: { scope: GlossaryScope; queue: GlossaryEntryUpdateQueue }
    nextOwner = {
      scope: ownerScope,
      queue: createGlossaryEntryUpdateQueue({
        initialRevision: revision,
        execute: (expectedRevision, id, patch) =>
          commands.updateGlossaryEntry(expectedRevision, id, patch),
        onResponse: async (view, current) => {
          if (current && owner.current === nextOwner) {
            await updateGlossaryCaches(scope, ownerScope, view)
          }
        },
      }),
    }
    owner.current = nextOwner
  }
  useEffect(() => owner.current?.queue.setRevision(revision), [revision])

  return useMutation({
    mutationKey: ['glossary', projectName, 'update'],
    mutationFn: async ({ id, patch }: { id: string; patch: GlossaryEntryPatch }) => {
      const mutationOwner = owner.current!
      try {
        return await mutationOwner.queue.enqueue(id, patch)
      } catch (error) {
        if (owner.current === mutationOwner) {
          await recoverGlossaryMutation(scope, mutationOwner.scope, error)
        }
        throw error
      }
    },
  })
}

export function useDeleteGlossaryEntries(projectName: string | undefined) {
  const scope = useGlossaryScope(projectName)
  return useMutation({
    mutationKey: ['glossary', projectName, 'delete'],
    mutationFn: ({ revision, ids }: { revision: number; ids: string[] }) =>
      commands.deleteGlossaryEntries(revision, ids),
    onMutate: () => captureGlossaryScope(scope),
    onSuccess: (view, _, captured) => updateGlossaryCaches(scope, captured, view),
    onError: (error, _, captured) => recoverGlossaryMutation(scope, captured, error),
  })
}

export function useTranslateGlossaryEntries(projectName: string | undefined) {
  const scope = useGlossaryScope(projectName)
  return useMutation({
    mutationKey: ['glossary', projectName, 'translate'],
    mutationFn: ({ revision, ids }: { revision: number; ids: string[] | null }) =>
      commands.translateGlossaryEntries(revision, ids),
    meta: { activity: 'glossary.translating' },
    onMutate: () => captureGlossaryScope(scope),
    onSuccess: (_, __, captured) => updateGlossaryCaches(scope, captured),
    onError: (error, _, captured) => recoverGlossaryMutation(scope, captured, error),
  })
}

export function usePreviewGlossaryImport(projectName: string | undefined) {
  return useMutation({
    mutationKey: ['glossary', projectName, 'import-preview'],
    mutationFn: (document: string) => commands.previewGlossaryImport(document),
  })
}

export function useApplyGlossaryImport(projectName: string | undefined) {
  const scope = useGlossaryScope(projectName)
  return useMutation({
    mutationKey: ['glossary', projectName, 'import-apply'],
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
    }) => commands.applyGlossaryImport(revision, document, strategy, confirmLanguageMismatch),
    onMutate: () => captureGlossaryScope(scope),
    onSuccess: (view, _, captured) => updateGlossaryCaches(scope, captured, view),
    onError: (error, _, captured) => recoverGlossaryMutation(scope, captured, error),
  })
}

export function useExportGlossary(projectName: string | undefined) {
  const scope = useGlossaryScope(projectName)
  return useMutation({
    mutationKey: ['glossary', projectName, 'export'],
    mutationFn: () => commands.exportGlossary(),
    meta: { activity: 'glossary.exporting' },
    onMutate: () => captureGlossaryScope(scope),
    onError: (error, _, captured) => recoverGlossaryMutation(scope, captured, error),
  })
}

export async function refresh(...keys: QueryKey[]): Promise<void> {
  await Promise.all(keys.map((queryKey) => queryClient.invalidateQueries({ queryKey })))
}
