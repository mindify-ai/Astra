import { type Auth, AuthError, MockAuth, type Session } from '@astra/client'
import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { SignIn } from '../src'

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true })
  Object.assign(navigator, { clipboard: { writeText: vi.fn(async () => {}) } })
})
afterEach(() => vi.useRealTimers())

const flush = (ms: number) => act(() => vi.advanceTimersByTimeAsync(ms))

describe('SignIn', () => {
  it('shows the device code, opens GitHub and signs in once approved', async () => {
    const onSignedIn = vi.fn<(s: Session) => void>()
    const openUrl = vi.fn()
    render(<SignIn auth={new MockAuth(undefined, 2)} onSignedIn={onSignedIn} openUrl={openUrl} />)

    fireEvent.click(screen.getByRole('button', { name: 'Continue with GitHub →' }))
    expect(await screen.findByLabelText('Your code')).toHaveProperty('textContent', 'WDJB-MJHT')
    expect(screen.getByText('Waiting for you to approve Astra on GitHub')).toBeTruthy()
    expect(screen.getByText('Code expires in 15:00')).toBeTruthy()

    fireEvent.click(screen.getByRole('button', { name: 'Open GitHub →' }))
    expect(openUrl).toHaveBeenCalledWith('https://github.com/login/device')
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith('WDJB-MJHT')

    await flush(2100)
    expect(onSignedIn).toHaveBeenCalledOnce()
    expect(onSignedIn.mock.calls[0]?.[0].person.name).toBe('Mark')
    expect(screen.getByText(/Signed in as Mark/)).toBeTruthy()
  })

  it('explains when a provider is not set up on the server', async () => {
    const auth: Auth = {
      start: async () => {
        throw new AuthError('notConfigured')
      },
      poll: async () => ({ status: 'pending' }),
    }
    render(<SignIn auth={auth} onSignedIn={() => {}} />)
    fireEvent.click(screen.getByRole('button', { name: 'Continue with GitLab' }))
    expect((await screen.findByRole('alert')).textContent).toContain(
      'Sign-in with GitLab is not set up on this server yet.',
    )
    // The buttons stay available to try another way.
    expect(screen.getByRole('button', { name: 'Continue with GitHub →' })).toBeTruthy()
  })

  it('reports a denied request and can start again', async () => {
    const auth: Auth = {
      start: new MockAuth().start,
      poll: async () => ({ status: 'denied' }),
    }
    render(<SignIn auth={auth} onSignedIn={() => {}} />)
    fireEvent.click(screen.getByRole('button', { name: 'Continue with GitHub →' }))
    await screen.findByLabelText('Your code')
    await flush(1100)
    expect((await screen.findByRole('alert')).textContent).toContain('Sign-in was cancelled on GitHub.')
  })

  it('cancel goes back without signing in', async () => {
    const onSignedIn = vi.fn()
    render(<SignIn auth={new MockAuth(undefined, 1)} onSignedIn={onSignedIn} />)
    fireEvent.click(screen.getByRole('button', { name: 'Continue with GitHub →' }))
    await screen.findByLabelText('Your code')
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
    await flush(3000)
    expect(onSignedIn).not.toHaveBeenCalled()
    expect(screen.getByRole('button', { name: 'Continue with GitHub →' })).toBeTruthy()
  })

  it('offers developer sign-in only when enabled', async () => {
    const onSignedIn = vi.fn<(s: Session) => void>()
    const { rerender } = render(<SignIn auth={new MockAuth()} onSignedIn={onSignedIn} />)
    expect(screen.queryByLabelText('Developer name')).toBeNull()
    rerender(<SignIn auth={new MockAuth()} onSignedIn={onSignedIn} devAuth />)
    fireEvent.change(screen.getByLabelText('Developer name'), { target: { value: 'Lin' } })
    fireEvent.click(screen.getByRole('button', { name: 'Dev sign-in' }))
    await flush(0)
    expect(onSignedIn.mock.calls[0]?.[0].person.name).toBe('Lin')
  })
})
