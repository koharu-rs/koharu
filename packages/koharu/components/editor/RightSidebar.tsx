'use client'

import { BookOpenText, Bot, Layers3 } from 'lucide-react'
import { useTranslation } from 'react-i18next'

import { AgentPanel } from '@/components/editor/AgentPanel'
import { LayersPanel } from '@/components/editor/LayersPanel'
import { StoryPanel } from '@/components/editor/StoryPanel'
import { useProject } from '@/lib/queries'
import { useKoharuStore, type Panel } from '@/lib/store'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@koharu/ui/components/tabs'

/** The right pane: Layers, Story, and Agent as tabs, each its full height, as Photoshop groups its
 * panels. The agent's tab stays mounted, so its conversation outlives a switch, and starts over
 * with another project, as the desktop's conversation does. */
export function RightSidebar() {
  const { t } = useTranslation()
  const project = useProject().data?.name ?? null
  const panel = useKoharuStore((state) => state.panel)
  const showPanel = useKoharuStore((state) => state.showPanel)
  // The shown tab in the accent colour, hovered or not, its underline too.
  const tab =
    'h-7 flex-none gap-1 px-1.5 text-[10px] font-semibold after:bg-primary data-active:text-primary data-active:hover:text-primary [&_svg]:size-3'
  return (
    <aside className='flex h-full min-h-0 flex-col bg-[var(--surface-panel)]'>
      <Tabs
        value={panel}
        onValueChange={(next) => showPanel(next as Panel)}
        className='min-h-0 flex-1 gap-0'
      >
        <TabsList
          variant='line'
          className='h-10 w-full shrink-0 justify-start rounded-none border-b border-border/80 px-1'
        >
          <TabsTrigger value='layers' className={tab}>
            <Layers3 /> {t('layers.title')}
          </TabsTrigger>
          <TabsTrigger value='story' className={tab}>
            <BookOpenText /> {t('story.title')}
          </TabsTrigger>
          <TabsTrigger value='agent' className={tab}>
            <Bot /> {t('agent.title')}
          </TabsTrigger>
        </TabsList>
        <TabsContent value='layers' className='flex min-h-0 flex-col'>
          <LayersPanel />
        </TabsContent>
        <TabsContent value='story' className='flex min-h-0 flex-col'>
          <StoryPanel />
        </TabsContent>
        <TabsContent value='agent' keepMounted className='flex min-h-0 flex-col'>
          <AgentPanel key={project} />
        </TabsContent>
      </Tabs>
    </aside>
  )
}
