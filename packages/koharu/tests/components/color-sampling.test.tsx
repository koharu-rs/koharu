import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { ColorSamplingProvider, useColorSampling } from '@/components/controls/ColorSampling'
import { ColorWell } from '@/components/controls/ColorWell'
import { useColorHistory } from '@/lib/colorHistory'
import { useKoharuStore } from '@/lib/store'

describe('canvas color sampling', () => {
  beforeEach(() => {
    useKoharuStore.setState({ tool: 'text' })
    useColorHistory.setState({ colors: [] })
  })

  it('applies a canvas sample to the requesting color well and restores the previous tool', async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()

    render(
      <ColorSamplingProvider>
        <ColorWell value='#111111' onChange={onChange} />
        <CompleteSample />
      </ColorSamplingProvider>,
    )

    await user.click(screen.getByRole('button', { name: 'Brush color' }))
    await user.click(screen.getByRole('button', { name: 'Pick color from canvas' }))
    expect(useKoharuStore.getState().tool).toBe('color_picker')

    await user.click(screen.getByRole('button', { name: 'Complete sample' }))
    expect(onChange).toHaveBeenCalledWith('#123456')
    expect(useKoharuStore.getState().tool).toBe('text')
  })

  it('commits shorthand white as a full RGB color', async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(<ColorWell value='#111111' onChange={onChange} />)

    await user.click(screen.getByRole('button', { name: 'Brush color' }))
    fireEvent.change(screen.getByRole('textbox', { name: 'Hex color code' }), {
      target: { value: '#FFF' },
    })

    expect(onChange).toHaveBeenCalledWith('#FFFFFF')
  })

  it('takes a hex code typed digit by digit past its valid shorthand', async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(<ColorWell value='#111111' onChange={onChange} />)

    await user.click(screen.getByRole('button', { name: 'Brush color' }))
    const hex = screen.getByRole('textbox', { name: 'Hex color code' })
    await user.clear(hex)
    await user.type(hex, 'ff0000')

    expect(onChange).toHaveBeenLastCalledWith('#FF0000')
    expect(hex).toHaveValue('#ff0000')
  })

  it('commits the final picker color when its window-level drag ends', async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(<ColorWell value='#111111' onChange={onChange} />)

    await user.click(screen.getByRole('button', { name: 'Brush color' }))
    const picker = screen.getByRole('slider', { name: 'Color' })
    Object.defineProperty(picker, 'getBoundingClientRect', {
      value: () => ({ left: 0, top: 0, width: 200, height: 160 }),
    })
    fireEvent.mouseDown(picker, { buttons: 1, clientX: 0, clientY: 0 })
    await waitFor(() =>
      expect(picker).toHaveAttribute('aria-valuetext', 'Saturation 0%, Brightness 100%'),
    )
    fireEvent.mouseUp(window)

    await waitFor(() => expect(onChange).toHaveBeenCalledWith('#FFFFFF'))
  })

  it('remembers the color the popover closes on, not each one applied before it', async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    const { rerender } = render(<ColorWell value='#111111' onChange={onChange} />)

    await user.click(screen.getByRole('button', { name: 'Brush color' }))
    const hex = screen.getByRole('textbox', { name: 'Hex color code' })
    fireEvent.change(hex, { target: { value: '#FF0000' } })
    fireEvent.change(hex, { target: { value: '#00FF00' } })
    await user.keyboard('{Escape}')

    rerender(<ColorWell value='#0000FF' onChange={onChange} />)
    await user.click(screen.getByRole('button', { name: 'Brush color' }))
    expect(screen.getAllByRole('button', { name: /^Recent color/ })).toHaveLength(1)
    await user.click(screen.getByRole('button', { name: 'Recent color #00FF00' }))
    expect(onChange).toHaveBeenLastCalledWith('#00FF00')
  })
})

function CompleteSample() {
  const sampling = useColorSampling()
  return (
    <button type='button' onClick={() => sampling?.complete('#123456')}>
      Complete sample
    </button>
  )
}
