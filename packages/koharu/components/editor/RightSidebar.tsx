'use client'

import { Bot, BookOpenText, SlidersHorizontal } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { AgentPanel } from '@/components/editor/AgentPanel'
import { GlossaryPanel } from '@/components/editor/GlossaryPanel'
import { Inspector } from '@/components/editor/Inspector'
import { useProject } from '@/lib/queries'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@koharu/ui/components/tabs'

type RightPanel = 'properties' | 'glossary' | 'agent'

export function RightSidebar() {
  const { t } = useTranslation()
  const project = useProject().data
  const [panel, setPanel] = useState<RightPanel>('properties')

  useEffect(() => {
    if (!project && panel === 'glossary') setPanel('properties')
  }, [panel, project])

  return (
    <aside className='flex h-full min-h-0 flex-col bg-[var(--surface-panel)]'>
      <Tabs
        value={panel}
        onValueChange={(value) => value && setPanel(value as RightPanel)}
        className='h-full min-h-0 gap-0'
      >
        <div className='flex h-10 shrink-0 items-center border-b border-border/80 px-2.5'>
          <TabsList variant='default' className='h-7 w-full p-0.5'>
            <TabsTrigger value='properties' className='text-[9px]'>
              <SlidersHorizontal className='size-3' /> {t('agent.properties')}
            </TabsTrigger>
            <TabsTrigger value='glossary' className='text-[9px]' disabled={!project}>
              <BookOpenText className='size-3' /> {t('glossary.title')}
            </TabsTrigger>
            <TabsTrigger value='agent' className='text-[9px]'>
              <Bot className='size-3' /> {t('agent.title')}
            </TabsTrigger>
          </TabsList>
        </div>
        <TabsContent value='properties' className='min-h-0 overflow-hidden'>
          <Inspector />
        </TabsContent>
        <TabsContent value='glossary' className='min-h-0 overflow-hidden'>
          <GlossaryPanel />
        </TabsContent>
        <TabsContent value='agent' className='min-h-0 overflow-hidden'>
          <AgentPanel />
        </TabsContent>
      </Tabs>
    </aside>
  )
}
