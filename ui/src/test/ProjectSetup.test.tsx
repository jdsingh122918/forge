import { describe, it, expect, beforeAll, afterEach, afterAll } from 'vitest'
import { render, screen, waitFor } from '@testing-library/react'
import { ProjectSetup } from '../components/ProjectSetup'
import { http, HttpResponse } from 'msw'
import { setupServer } from 'msw/node'

const server = setupServer(
  http.get('/api/github/status', () => {
    return HttpResponse.json({ connected: false, client_id_configured: true })
  }),
)

beforeAll(() => server.listen())
afterEach(() => server.resetHandlers())
afterAll(() => server.close())

describe('ProjectSetup', () => {
  it('shows device flow button when client_id is configured', async () => {
    render(
      <ProjectSetup
        projects={[]}
        onSelect={() => {}}
        onCreate={() => {}}
        onClone={async () => {}}
      />
    )

    await waitFor(() => {
      expect(screen.getByText('Sign in with GitHub')).toBeInTheDocument()
    })
  })

  it('shows PAT input when client_id is not configured', async () => {
    server.use(
      http.get('/api/github/status', () => {
        return HttpResponse.json({ connected: false, client_id_configured: false })
      }),
    )

    render(
      <ProjectSetup
        projects={[]}
        onSelect={() => {}}
        onCreate={() => {}}
        onClone={async () => {}}
      />
    )

    await waitFor(() => {
      expect(screen.getByLabelText('Personal access token')).toBeInTheDocument()
    })
  })

  it('shows PAT toggle link when device flow is available', async () => {
    render(
      <ProjectSetup
        projects={[]}
        onSelect={() => {}}
        onCreate={() => {}}
        onClone={async () => {}}
      />
    )

    await waitFor(() => {
      expect(screen.getByText('Or use a personal access token')).toBeInTheDocument()
    })
  })
})
