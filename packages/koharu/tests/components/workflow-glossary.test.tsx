import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { ProjectGlossaryDialog } from '@/components/app/ProjectGlossaryDialog'
import { WorkflowDialog } from '@/components/app/WorkflowDialog'
import { ActivityCenter } from '@/components/editor/ActivityCenter'
import { useKoharuStore } from '@/lib/store'
import { commands, type Job, type WorkflowPreset } from '@koharu/bridge/protocol'

const consistent: WorkflowPreset = {
  name: 'Consistent Project Translation',
  scheduling: 'stage_major',
  review_glossary: true,
  stages: ['detection', 'ocr', 'terminology', 'translation', 'inpainting'],
}
const waiting: Job = {
  id: 'workflow',
  kind: 'workflow',
  state: 'awaiting_review',
  completed: 60,
  total: 100,
  page: null,
  stage: null,
  model: null,
  error: null,
  workflow: {
    name: 'Consistent Project Translation',
    stages: [
      { stage: 'detection', state: 'complete', completed: 20, total: 20 },
      { stage: 'ocr', state: 'complete', completed: 20, total: 20 },
      { stage: 'terminology', state: 'awaiting_review', completed: 20, total: 20 },
      { stage: 'translation', state: 'pending', completed: 0, total: 20 },
    ],
  },
}

describe('project workflows and glossary', () => {
  it('selects a saved scheduling mode and starts only selected pages', async () => {
    vi.spyOn(commands, 'getWorkflowPresets').mockResolvedValue([
      {
        name: 'Standard',
        scheduling: 'page_major',
        stages: ['detection', 'ocr', 'translation', 'inpainting'],
        review_glossary: true,
      },
      consistent,
    ])
    const start = vi.spyOn(commands, 'startWorkflow').mockResolvedValue('workflow')
    useKoharuStore.setState({ selectedPages: ['page-two', 'page-four'] })
    render(<WorkflowDialog open onOpenChange={vi.fn()} />)
    expect(await screen.findByLabelText('Scheduling')).toHaveValue('stage_major')
    fireEvent.change(screen.getByLabelText('Scheduling'), { target: { value: 'page_major' } })
    expect(screen.getByLabelText('Terminology analysis')).not.toBeChecked()
    expect(screen.getByLabelText('Terminology analysis')).toBeDisabled()
    fireEvent.change(screen.getByLabelText('Preset'), { target: { value: consistent.name } })
    fireEvent.change(screen.getByLabelText('Scope'), { target: { value: 'selected_pages' } })
    fireEvent.click(screen.getByRole('button', { name: 'Start workflow' }))
    await waitFor(() =>
      expect(start).toHaveBeenCalledWith(
        { scope: 'pages', value: ['page-two', 'page-four'] },
        { ...consistent, scope: 'selected_pages' },
      ),
    )
  })

  it('confirms a candidate, edits its translation, saves, then resumes the same job', async () => {
    const document = {
      project: 'project',
      glossary: {
        entries: [],
        ignored: [],
        candidates: [
          {
            source: '高橋',
            suggested_target: '',
            category: 'person' as const,
            occurrences: 12,
            page_count: 8,
          },
        ],
      },
    }
    vi.spyOn(commands, 'getProjectGlossary').mockResolvedValue(document)
    const save = vi
      .spyOn(commands, 'saveProjectGlossary')
      .mockImplementation(async (_, glossary) => ({ project: 'project', glossary }))
    const resume = vi.spyOn(commands, 'resumeWorkflow').mockResolvedValue(null)
    useKoharuStore.setState({ jobs: { workflow: waiting } })
    render(<ProjectGlossaryDialog open onOpenChange={vi.fn()} />)
    expect(await screen.findByLabelText('Source 1')).toHaveValue('高橋')
    expect(screen.getByRole('button', { name: 'Confirm' })).toBeDisabled()
    fireEvent.change(screen.getByLabelText('Translation 1'), { target: { value: '高桥' } })
    fireEvent.click(screen.getByRole('button', { name: 'Confirm' }))
    fireEvent.click(screen.getByRole('button', { name: 'Confirmed (1)' }))
    fireEvent.change(screen.getByLabelText('Notes 1'), { target: { value: 'Protagonist surname' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save and continue translation' }))
    await waitFor(() => expect(resume).toHaveBeenCalledWith('workflow'))
    expect(save).toHaveBeenCalledWith(document, {
      entries: [
        {
          source: '高橋',
          target: '高桥',
          category: 'person',
          notes: 'Protagonist surname',
          enabled: true,
        },
      ],
      candidates: [],
      ignored: [],
    })
    expect(save.mock.invocationCallOrder[0]).toBeLessThan(resume.mock.invocationCallOrder[0])
  })

  it('exposes review and cancellation while displaying all stage states', async () => {
    const stop = vi.spyOn(commands, 'stopJob').mockResolvedValue(null)
    useKoharuStore.setState({ jobs: { workflow: waiting } })
    render(<ActivityCenter />)
    expect(screen.getByText('OCR')).toBeInTheDocument()
    expect(screen.getByText('20 / 20 · Review required')).toBeInTheDocument()
    expect(screen.getByText('0 / 20 · Pending')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Review glossary' }))
    expect(useKoharuStore.getState().glossaryOpen).toBe(true)
    fireEvent.click(screen.getByRole('button', { name: 'Stop' }))
    await waitFor(() => expect(stop).toHaveBeenCalledWith('workflow'))
  })
})
