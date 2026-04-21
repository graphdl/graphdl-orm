/**
 * ErrorBoundary — React error boundary specialised for AREST views.
 *
 * React only surfaces render-time errors through class boundaries.
 * This wraps the standard componentDidCatch pattern and offers a
 * reset callback so callers can retry without remounting the tree.
 *
 * Pair with the query-layer error handling: mutation errors surface
 * through `useMutation`'s error state and don't need boundaries;
 * boundaries catch unhandled render errors (schema walking a null,
 * etc.) that would otherwise crash the tab.
 */
import { Component, type ErrorInfo, type ReactNode } from 'react'

export interface ErrorBoundaryProps {
  children: ReactNode
  /** Custom fallback. Receives the caught error and a reset fn. */
  fallback?: (error: Error, reset: () => void) => ReactNode
  /** Called on every caught error — good for telemetry. */
  onError?: (error: Error, info: ErrorInfo) => void
}

interface ErrorBoundaryState {
  error: Error | null
}

export class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  state: ErrorBoundaryState = { error: null }

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { error }
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    this.props.onError?.(error, info)
  }

  private reset = (): void => {
    this.setState({ error: null })
  }

  render(): ReactNode {
    if (this.state.error) {
      if (this.props.fallback) return this.props.fallback(this.state.error, this.reset)
      return (
        <div role="alert" data-testid="error-boundary" style={{ padding: '1rem', border: '1px solid crimson', borderRadius: 6 }}>
          <p><strong>Something went wrong.</strong></p>
          <pre style={{ whiteSpace: 'pre-wrap', fontSize: '0.875rem' }}>{this.state.error.message}</pre>
          <button type="button" onClick={this.reset} data-testid="error-boundary-reset">Try again</button>
        </div>
      )
    }
    return this.props.children
  }
}
