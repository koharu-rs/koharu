'use client'

import { BookOpenText, Bot, Layers3 } from 'lucide-react'
import { useHotkeys } from 'react-hotkeys-hook'
import { useTranslation } from 'react-i18next'

import { AgentPanel } from '@/components/editor/AgentPanel'
import { LayersPanel } from '@/components/editor/LayersPanel'
import { StoryPanel } from '@/components/editor/StoryPanel'
import { shortcutLabel, shortcuts } from '@/lib/shortcuts'
import { useKoharuStore, type Panel } from '@/lib/store'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@koharu/ui/components/tabs'
import { Tooltip, TooltipContent, TooltipTrigger } from '@koharu/ui/components/tooltip'

const tabs = [
  ['layers', Layers3],
  ['story', BookOpenText],
  ['agent', Bot],
] as const satisfies ReadonlyArray<readonly [Panel, typeof Layers3]>

/** The right pane: Layers, Story, and Agent as tabs, each its full height, as Photoshop groups its
 * panels. The agent's tab stays mounted, so a switch leaves its transcript where it was. */
export function RightSidebar() {
  const { t } = useTranslation()
  const panel = useKoharuStore((state) => state.panel)
  const showPanel = useKoharuStore((state) => state.showPanel)
  // Photoshop's key for Layers, which shows Story in its place when it's in front. It works from a
  // field too, where it types nothing: an input method's own F7 doesn't come as that key.
  const next = panel === 'layers' ? 'story' : 'layers'
  useHotkeys(shortcuts.layers, () => showPanel(next), {
    preventDefault: true,
    useKey: true,
    enableOnFormTags: true,
  })
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
          {tabs.map(([value, Icon]) => (
            <Tooltip key={value} disabled={value !== next}>
              <TooltipTrigger render={<TabsTrigger value={value} className={tab} />}>
                <Icon /> {t(`${value}.title`)}
              </TooltipTrigger>
              <TooltipContent side='bottom'>
                {t(`${value}.title`)}
                <span className='ml-2 opacity-60'>{shortcutLabel(shortcuts.layers)}</span>
              </TooltipContent>
            </Tooltip>
          ))}
        </TabsList>
        <TabsContent value='layers' className='flex min-h-0 flex-col'>
          <LayersPanel />
        </TabsContent>
        <TabsContent value='story' className='flex min-h-0 flex-col'>
          <StoryPanel />
        </TabsContent>
        <TabsContent value='agent' keepMounted className='flex min-h-0 flex-col'>
          <AgentPanel />
        </TabsContent>
      </Tabs>
    </aside>
  )
}
