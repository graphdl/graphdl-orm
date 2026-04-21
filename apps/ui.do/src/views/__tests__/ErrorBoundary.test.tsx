import { describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'
import { useState } from 'react'
import { ErrorBoundary } from '../ErrorBoundary'

function Bomb({ fuse }: { fuse: boolean }) {
  if (fuse) throw new Error('boom')
  return <p data-testid="safe">safe</p>
}

describe('ErrorBoundary', () => {
  it('renders children when no error', () => {
    render(<ErrorBoundary><Bomb fuse={false} /></ErrorBoundary>)
    expect(screen.getByTestId('safe')).toBeDefined()
  })

  it('catches render-time errors and renders the default fallback', () => {
    // React logs caught errors via console.error; silence it for a
    // deterministic test.
    const err = vi.spyOn(console, 'error').mockImplementation(() => {})
    render(<ErrorBoundary><Bomb fuse={true} /></ErrorBoundary>)
    expect(screen.getByTestId('error-boundary')).toBeDefined()
    expect(screen.getByTestId('error-boundary').textContent).toMatch(/boom/)
    err.mockRestore()
  })

  it('reset button clears the caught error so the tree can recover', () => {
    const err = vi.spyOn(console, 'error').mockImplementation(() => {})

    function Toggle() {
      const [fuse, setFuse] = useState(true)
      return (
        <ErrorBoundary
          fallback={(e, reset) => (
            <div>
              <span data-testid="msg">{e.message}</span>
              <button
                type="button"
                data-testid="recover"
                onClick={() => { setFuse(false); reset() }}
              >recover</button>
            </div>
          )}
        >
          <Bomb fuse={fuse} />
        </ErrorBoundary>
      )
    }

    render(<Toggle />)
    expect(screen.getByTestId('msg').textContent).toBe('boom')
    fireEvent.click(screen.getByTestId('recover'))
    expect(screen.getByTestId('safe')).toBeDefined()
    err.mockRestore()
  })

  it('calls onError with the caught error and info', () => {
    const err = vi.spyOn(console, 'error').mockImplementation(() => {})
    const onError = vi.fn()
    render(<ErrorBoundary onError={onError}><Bomb fuse={true} /></ErrorBoundary>)
    expect(onError).toHaveBeenCalledTimes(1)
    expect(onError.mock.calls[0][0]).toBeInstanceOf(Error)
    expect(onError.mock.calls[0][0].message).toBe('boom')
    err.mockRestore()
  })
})
