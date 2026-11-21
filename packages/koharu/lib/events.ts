'use client'

import { commands, events, type Change, type PageId } from '@koharu/ipc/protocol'
import { toast } from '@koharu/ui/components/toast'

import { toHex } from './color'
import i18n from './i18n'
import { pageKey, pagesKey, projectKey, queryClient, refresh, type ProjectView } from './queries'
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
    listen(events.pipeline, receivePipeline),
    listen(events.download, receiveDownload),
  ])
    .then(() => commands.getPipelineEvents())
    .then((runs) => {
      if (active()) receiveRuns(runs)
    })
}

/** Another project opened, or the open one closed: its selection starts empty. A project told
 * again, after changes the interface missed, is read again. */
function receiveProject(name: string | null): void {
  const previous = queryClient.getQueryData<ProjectView | null>(projectKey)
  if (previous?.name !== name) {
    const store = useKoharuStore.getState()
    store.selectPages([])
    store.selectLayers([])
  }
  if (name === null) {
    queryClient.setQueryData(projectKey, null)
    queryClient.setQueryData(pageKey, null)
  } else {
    void refresh(projectKey, pageKey).catch(() => undefined)
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
 * and so are the pages Story lists, which a change to one of their layers doesn't name. */
function receiveChange(change: Change): void {
  queryClient.setQueryData<ProjectView | null>(projectKey, (project) =>
    project
      ? {
          ...project,
          history: change.history,
          revisions: project.page
            ? { ...project.revisions, [project.page]: change.revision }
            : project.revisions,
        }
      : project,
  )
  void refresh(projectKey, pageKey, pagesKey).catch(() => undefined)
}
