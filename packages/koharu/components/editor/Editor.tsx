'use client'

import { ColorSamplingProvider } from '@/components/controls/ColorSampling'
import { ActivityCenter } from '@/components/editor/ActivityCenter'
import { CanvasWorkspace } from '@/components/editor/CanvasWorkspace'
import { PageRail } from '@/components/editor/PageRail'
import { Reader } from '@/components/editor/Reader'
import { RightSidebar } from '@/components/editor/RightSidebar'
import { useKoharuStore } from '@/lib/store'
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
  useDefaultLayout,
} from '@koharu/ui/components/resizable'

/** The editor: the page list and the right pane keep their widths in pixels, as Photoshop's and
 * Clip Studio Paint's panels do, so a larger window's width goes to the canvas; the widths a user
 * drags to are remembered. Reading shows the reader alone, without either. */
export function Editor() {
  const reading = useKoharuStore((state) => state.reading)
  const layout = useDefaultLayout({
    id: 'editor',
    storage: typeof window === 'undefined' ? undefined : window.localStorage,
    onlySaveAfterUserInteractions: true,
  })
  return (
    <ColorSamplingProvider>
      <div className='relative min-h-0 flex-1 bg-transparent'>
        <ActivityCenter />
        {reading ? (
          <Reader />
        ) : (
          <ResizablePanelGroup
            id='editor'
            orientation='horizontal'
            className='h-full min-h-0'
            defaultLayout={layout.defaultLayout}
            onLayoutChanged={layout.onLayoutChanged}
          >
            <ResizablePanel
              id='pages'
              defaultSize='160px'
              minSize='160px'
              maxSize='320px'
              collapsible
              collapsedSize='48px'
              groupResizeBehavior='preserve-pixel-size'
              className='min-h-0 overflow-hidden'
            >
              <PageRail />
            </ResizablePanel>
            <ResizableHandle className='w-0 bg-transparent' />
            <ResizablePanel
              id='canvas'
              minSize='420px'
              className='workspace-corner-mask relative z-10 min-h-0 rounded-tl-2xl bg-transparent shadow-[var(--shadow-content)]'
            >
              <CanvasWorkspace />
            </ResizablePanel>
            <ResizableHandle className='w-0 bg-transparent' />
            <ResizablePanel
              id='inspector'
              defaultSize='340px'
              minSize='280px'
              maxSize='480px'
              collapsible
              collapsedSize='48px'
              groupResizeBehavior='preserve-pixel-size'
              className='min-h-0 overflow-hidden border-l border-border/40 bg-[var(--surface-panel)]'
            >
              <RightSidebar />
            </ResizablePanel>
          </ResizablePanelGroup>
        )}
      </div>
    </ColorSamplingProvider>
  )
}
