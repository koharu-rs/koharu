import { QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { StartView } from '@/components/start/StartView'
import { queryClient, useProject } from '@/lib/queries'
import { commands } from '@koharu/bridge/protocol'

function ProjectFlow() {
  const project = useProject().data
  return project ? <p>Opened {project.name}</p> : <StartView />
}

/** The desktop with `open` as its open project, empty. */
function desktop(open: () => string | null) {
  vi.spyOn(commands, 'getOpenProject').mockImplementation(async () => open())
  vi.spyOn(commands, 'getShownPage').mockResolvedValue(null)
  vi.spyOn(commands, 'query').mockImplementation(async (query) =>
    query.query === 'get_project'
      ? { id: 'project', version: '1', pages: [] }
      : { steps: [], current: 0 },
  )
}

function renderProjectFlow() {
  return render(
    <QueryClientProvider client={queryClient}>
      <ProjectFlow />
    </QueryClientProvider>,
  )
}

describe('StartView', () => {
  afterEach(() => vi.restoreAllMocks())

  it('creates a managed project by name', async () => {
    let opened = false
    desktop(() => (opened ? 'Volume 1' : null))
    vi.spyOn(commands, 'listProjects').mockResolvedValue([])
    const create = vi.spyOn(commands, 'createProject').mockImplementation(async () => {
      opened = true
      return null
    })
    renderProjectFlow()
    expect(await screen.findByRole('heading', { name: 'Projects' })).toBeInTheDocument()
    fireEvent.change(screen.getByRole('textbox', { name: 'Project name' }), {
      target: { value: 'Volume 1' },
    })
    const createButton = screen.getByRole('button', { name: 'Create' })
    await waitFor(() => expect(createButton).toBeEnabled())
    fireEvent.click(createButton)
    await waitFor(() => expect(create).toHaveBeenCalledWith('Volume 1'))
    expect(await screen.findByText('Opened Volume 1')).toBeInTheDocument()
  })

  it('opens a managed project without reloading the application', async () => {
    let opened = false
    desktop(() => (opened ? 'Blue Archive' : null))
    vi.spyOn(commands, 'listProjects').mockResolvedValue(['Blue Archive'])
    const open = vi.spyOn(commands, 'openProject').mockImplementation(async () => {
      opened = true
      return null
    })
    renderProjectFlow()

    const project = (await screen.findByText('Blue Archive')).closest('button')
    if (!project) throw new Error('project row is not interactive')
    await waitFor(() => expect(project).toBeEnabled())
    fireEvent.click(project)

    await waitFor(() => expect(open).toHaveBeenCalledWith('Blue Archive'))
    expect(await screen.findByText('Opened Blue Archive')).toBeInTheDocument()
  })

  it('confirms before deleting a managed project', async () => {
    vi.spyOn(commands, 'listProjects')
      .mockResolvedValueOnce(['Blue Archive'])
      .mockResolvedValueOnce([])
    const remove = vi.spyOn(commands, 'deleteProject').mockResolvedValue(null)
    render(<StartView />)

    const deleteButton = await screen.findByRole('button', { name: 'Delete Blue Archive' })
    await waitFor(() => expect(deleteButton).toBeEnabled())
    fireEvent.click(deleteButton)

    expect(screen.getByRole('alertdialog')).toHaveTextContent(
      'This permanently deletes “Blue Archive” and all of its pages.',
    )
    expect(remove).not.toHaveBeenCalled()

    fireEvent.click(screen.getByRole('button', { name: 'Delete project' }))
    await waitFor(() => expect(remove).toHaveBeenCalledWith('Blue Archive'))
  })
})
