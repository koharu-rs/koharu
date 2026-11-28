'use client'

import { commands, events, type Change, type Loading, type PageId } from '@koharu/ipc/protocol'
import { toast } from '@koharu/ui/components/toast'

import { useConversation } from './agent'
import { toHex } from './color'
import { receivePainter } from './formatPainter'
import i18n from './i18n'
import {
  pageKey,
  pagesKey,
  projectKey,
  queryClient,
  refresh,
  thumbnailKey,
  type ProjectView,
} from './queries'
import {
  receiveCanvasSelection,
  receiveDownload,
  receivePickedColor,
  receivePipeline,
  receiveRuns,
  useKoharuStore,
} from './store'

/** Hears what the desktop tells for the interface's life, while `active` holds, which it doesn't
 * while React replays an effect: listening first, then reading the runs so far, so nothing told
 * while they load is missed. */
export function hear(active: () => boolean): Promise<void> {
  const listen = <T>(
    event: (handler: (payload: T) => void) => Promise<unknown>,
    receive: (payload: T) => void,
  ) =>
    event((payload) => {
      if (active()) receive(payload)
    })

  return Promise.all([
    listen(events.change, receiveChange),
    listen(events.project, receiveProject),
    listen(events.page, receivePage),
    listen(events.selection, receiveCanvasSelection),
    listen(events.camera, (camera) => useKoharuStore.setState({ camera })),
    listen(events.selectionBounds, (selectionBounds) =>
      useKoharuStore.setState({ selectionBounds }),
    ),
    listen(events.area, (area) => useKoharuStore.setState({ area })),
    listen(events.typing, (typing) => useKoharuStore.setState({ typing })),
    listen(events.color, (color) => receivePickedColor(toHex(color))),
    listen(events.brushDiameter, (diameter) => {
      const store = useKoharuStore.getState()
      store.setBrush({ ...store.brush, diameter })
    }),
    listen(events.notice, (notice) =>
      toast.add({ type: 'info', title: i18n.t(`canvas.notice.${notice}`) }),
    ),
    listen(events.painter, receivePainter),
    listen(events.pipeline, receivePipeline),
    listen(events.download, receiveDownload),
    listen(events.chapter, receiveChapter),
  ])
    .then(() => commands.getPipelineEvents())
    .then((runs) => {
      if (active()) receiveRuns(runs)
    })
}

/** An extension's chapter loading into the open project went further, or failed. */
function receiveChapter(loading: Loading): void {
  useKoharuStore.setState({ loading })
  if (loading.error)
    toast.add({
      type: 'error',
      title: i18n.t('reader.loadFailed'),
      description: loading.error,
    })
}

/** Another project opened, or the open one closed: its selection and its conversation start empty,
 * and no thumbnail is kept, since its pages' revisions aren't. A project told again, after changes
 * the interface missed, is read again, its thumbnails too. */
function receiveProject(name: string | null): void {
  const previous = queryClient.getQueryData<ProjectView | null>(projectKey)
  if (previous?.name !== name) {
    const store = useKoharuStore.getState()
    store.selectPages([])
    store.selectLayers([])
    useConversation.setState(useConversation.getInitialState())
    // A chapter loads only into the project that's open.
    useKoharuStore.setState({ loading: null })
    queryClient.removeQueries({ queryKey: thumbnailKey })
  }
  if (name === null) {
    queryClient.setQueryData(projectKey, null)
    queryClient.setQueryData(pageKey, null)
  } else {
    void refresh(projectKey, pageKey, thumbnailKey).catch(() => undefined)
  }
}

/** The page the canvas shows. The project is read again too: a read already under way, which a
 * change starts, answers the page shown before, and would land over this one. */
function receivePage(page: PageId | null): void {
  queryClient.setQueryData<ProjectView | null>(projectKey, (project) =>
    project ? { ...project, page } : project,
  )
  if (page !== null && useKoharuStore.getState().selectedPages.length === 0) {
    useKoharuStore.getState().selectPages([page])
  }
  void refresh(projectKey, pageKey).catch(() => undefined)
}

/** An edit, undo, or redo, from whichever writer: the project and its shown page are read again,
 * and so are the pages Story lists. */
function receiveChange(change: Change): void {
  const named = [...change.pages.added, ...change.pages.changed]
  queryClient.setQueryData<ProjectView | null>(projectKey, (project) =>
    project
      ? {
          ...project,
          history: change.history,
          revisions: {
            ...project.revisions,
            ...Object.fromEntries(named.map((page) => [page, change.revision])),
          },
        }
      : project,
  )
  // Undo and redo bring a revision back, but not always its thumbnail: the desktop draws the page
  // as it is when the read gets there, which can be after a later change.
  queryClient.removeQueries({
    queryKey: thumbnailKey,
    type: 'inactive',
    predicate: ({ queryKey }) => named.includes(queryKey[1] as PageId),
  })
  void refresh(projectKey, pageKey, pagesKey).catch(() => undefined)
}
