'use client'

import {
  MutationCache,
  MutationObserver,
  QueryCache,
  QueryClient,
  type QueryKey,
  queryOptions,
  useIsMutating,
  useMutation,
  useQuery,
} from '@tanstack/react-query'
import type { ParseKeys } from 'i18next'
import { useDebouncedCallback } from 'use-debounce'

import {
  commands,
  type FontFamily,
  type History,
  type LayerId,
  type Page,
  type PageId,
  type PageSummary,
  type Preferences,
  type ProjectAnswer,
} from '@koharu/ipc/protocol'
import { toast } from '@koharu/ui/components/toast'

import i18n from './i18n'

export const projectKey = ['project'] as const
export const pageKey = ['page'] as const
/** Each page Story lists, by its id, whether or not the canvas shows it. */
export const pagesKey = ['pages'] as const
/** Each page's thumbnails, by its id and what they show. */
export const thumbnailKey = ['thumbnail'] as const
export const projectsKey = ['projects'] as const
export const fontsKey = ['fonts'] as const
export const preferencesKey = ['preferences'] as const
export const modelsKey = ['models'] as const

/** The open project as the editor lists it: its name, the page the canvas shows, its pages, and
 * its history, read through the scene's own queries. */
export interface ProjectView {
  name: string
  page: PageId | null
  pages: PageSummary[]
  history: History
  /** What the project holds beyond its pages, such as the faces it sets. */
  meta: ProjectAnswer['meta']
  /** Each page's revision when a change last named it, added or changed, which its thumbnail
   * follows whichever page is shown. */
  revisions: Record<PageId, string>
}

declare module '@tanstack/react-query' {
  interface Register {
    /** `activity` is the activity center's row while the mutation runs. */
    mutationMeta: { activity?: ParseKeys }
  }
}

/** A failed command's toast: the desktop rejects with its error's message. */
function report(error: unknown): void {
  const description =
    error instanceof Error
      ? error.message
      : typeof error === 'string'
        ? error
        : i18n.t('errors.unknown')
  toast.add({ type: 'error', title: i18n.t('errors.actionFailed'), description })
}

/** Every command runs as a query or a mutation, whose failure shows here. Commands go to the
 * desktop, not the network, so they run whether or not the browser reports being online. */
export const queryClient = new QueryClient({
  queryCache: new QueryCache({ onError: report }),
  mutationCache: new MutationCache({ onError: report }),
  defaultOptions: {
    queries: {
      networkMode: 'always',
      staleTime: Number.POSITIVE_INFINITY,
      retry: false,
      refetchOnReconnect: false,
      refetchOnWindowFocus: false,
    },
    mutations: { networkMode: 'always' },
  },
})

/** Runs a command whose answer nothing waits on, as a mutation, so a failure shows as any
 * mutation's does. */
export function send<Args extends unknown[]>(
  command: (...args: Args) => Promise<unknown>,
  ...args: Args
): void {
  // TanStack Query never drops a mutation while an observer holds it, and nothing reads a
  // settled command, so let go of it once it settles and let the cache drop it right away.
  const observer = new MutationObserver(queryClient, {
    mutationFn: () => command(...args),
    gcTime: 0,
  })
  void observer
    .mutate()
    .catch(() => undefined)
    .finally(() => observer.reset())
}

const projectQuery = queryOptions({
  queryKey: projectKey,
  queryFn: async (): Promise<ProjectView | null> => {
    const name = await commands.getOpenProject()
    if (name === null) return null
    const [page, project, history] = await Promise.all([
      commands.getShownPage(),
      commands.query({ query: 'get_project' }),
      commands.query({ query: 'get_history' }),
    ])
    const known = queryClient.getQueryData<ProjectView | null>(projectKey)
    return {
      name,
      page,
      pages: (project as ProjectAnswer).pages,
      history: history as History,
      meta: (project as ProjectAnswer).meta,
      revisions: known?.name === name ? known.revisions : {},
    }
  },
})

const pageQuery = queryOptions({
  queryKey: pageKey,
  queryFn: async (): Promise<Page | null> => {
    const page = await commands.getShownPage()
    if (page === null) return null
    return (await commands.query({ query: 'get_page', page })) as Page
  },
})

const fontsQuery = queryOptions({
  queryKey: fontsKey,
  queryFn: () => commands.listFonts(),
})

/** The open project, read again after every change the desktop tells. */
export function useProject(enabled = true) {
  return useQuery({ ...projectQuery, enabled })
}

/** The open project's pages, in order. */
export function usePages(enabled = true) {
  return { data: useProject(enabled).data?.pages }
}

/** The page the editor shows, with its layers. */
export function usePage(enabled = true) {
  return useQuery({ ...pageQuery, enabled })
}

/** A page with its layers, and its texts in the scene's reading order, as Story lists them. */
export function usePageStory(page: PageId, enabled = true) {
  return useQuery({
    queryKey: [...pagesKey, page],
    queryFn: async () => {
      const [layers, story] = await Promise.all([
        commands.query({ query: 'get_page', page }),
        commands.query({ query: 'get_story', page }),
      ])
      return { page: layers as Page, story: story as LayerId[] }
    },
    enabled,
  })
}

export function useFonts(enabled = true) {
  return useQuery({ ...fontsQuery, enabled })
}

/** The preferences as the desktop last read or saved them, which a failed save shows again: a
 * file that can't be written may not read either. */
let stored: Preferences | undefined

export function usePreferences() {
  return useQuery({
    queryKey: preferencesKey,
    queryFn: async () => (stored = await commands.getPreferences()),
    // The cache keeps the very object a save sent, which the save is known by.
    structuralSharing: false,
  })
}

/** Saves the preferences one save after another. Each save shows at once; what the file holds
 * after it, or held before it if it failed, replaces it only while the cache still holds that
 * save, so an answer never undoes a newer edit, saved or still waiting to be. */
export function useSavePreferences() {
  return useMutation({
    mutationKey: preferencesKey,
    scope: { id: 'preferences' },
    mutationFn: (preferences: Preferences) => commands.savePreferences(preferences),
    onMutate: async (preferences) => {
      await queryClient.cancelQueries({ queryKey: preferencesKey })
      queryClient.setQueryData(preferencesKey, preferences)
    },
    onSettled: (saved, _, preferences) => {
      stored = saved ?? stored
      if (queryClient.getQueryData(preferencesKey) === preferences) {
        queryClient.setQueryData(preferencesKey, stored)
      }
    },
  })
}

/** Edits the preferences as a settings page does: each edit shows at once, and saves once the
 * edits pause; `flush` saves one still waiting. */
export function useEditPreferences() {
  const { mutate } = useSavePreferences()
  const save = useDebouncedCallback(mutate, 260, { flushOnExit: true })
  const edit = (preferences: Preferences) => {
    void queryClient.cancelQueries({ queryKey: preferencesKey })
    queryClient.setQueryData(preferencesKey, preferences)
    save(preferences)
  }
  return { edit, flush: save.flush }
}

/** The models the translator and the agent can use. */
export function useModels() {
  return useQuery({ queryKey: modelsKey, queryFn: () => commands.listModels() })
}

/** The projects the start view lists, read again each time it shows. */
export function useProjects() {
  return useQuery({
    queryKey: projectsKey,
    queryFn: () => commands.listProjects(),
    refetchOnMount: 'always',
  })
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

/** A command the activity center lists by `label` while it runs, busy wherever it's used. */
export function useCommand<Args extends unknown[], Result>(
  key: QueryKey,
  command: (...args: Args) => Promise<Result>,
  label: ParseKeys,
) {
  const busy = useIsMutating({ mutationKey: key }) > 0
  const mutation = useMutation({
    mutationKey: key,
    mutationFn: (args: Args) => command(...args),
    meta: { activity: label },
  })
  return { run: (...args: Args) => mutation.mutate(args), busy }
}

/** Imports pages; the project's change arrives as an event. */
export function useImportPages() {
  const { run, busy } = useCommand(['import-pages'], commands.import, 'navigator.importing')
  return { importPages: run, importing: busy }
}

/** Resolves once the shown page has been read since the call: a read already under way, as a
 * change starts one, is joined rather than started again, and one a later change starts is waited
 * for too. An edit shown before it lands holds until then, so it doesn't flash back. */
export async function readPage(): Promise<void> {
  do await queryClient.refetchQueries({ queryKey: pageKey }, { cancelRefetch: false })
  while (queryClient.isFetching({ queryKey: pageKey }) > 0)
}

export async function refresh(...keys: QueryKey[]): Promise<void> {
  await Promise.all(keys.map((queryKey) => queryClient.invalidateQueries({ queryKey })))
}
