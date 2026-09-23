// @vitest-environment jsdom
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { mcpAddCommand } from '../agent';
import type { NewToken } from '../api';
import type { TokenInfo } from '../types';
import { AgentDialog } from './AgentDialog';

const ORIGIN = 'http://localhost:3000';

const { createToken, listTokens } = vi.hoisted(() => ({
  createToken: vi.fn<() => Promise<NewToken>>(),
  listTokens: vi.fn<() => Promise<TokenInfo[]>>(),
}));

vi.mock('../api', () => ({
  createToken,
  listTokens,
  revokeToken: vi.fn(),
}));

describe('AgentDialog', () => {
  it('shows the endpoint before any token is minted, with a placeholder in the snippets', async () => {
    listTokens.mockResolvedValue([]);
    render(<AgentDialog onCancel={vi.fn()} />);

    expect(screen.getByText('http://localhost:3000/mcp')).toBeTruthy();
    await waitFor(() => expect(screen.getByText(/no tokens yet/i)).toBeTruthy());

    // The Claude Code tab is active by default and still shows the placeholder.
    expect(screen.getByText(mcpAddCommand(ORIGIN, '<token>'))).toBeTruthy();
    expect(screen.getByText(/create a token above/i)).toBeTruthy();
  });

  it('fills the snippets with the real token once one is minted', async () => {
    const user = userEvent.setup();
    listTokens.mockResolvedValue([]);
    createToken.mockResolvedValue({
      token: 'tns_live123',
      id: 't1',
      label: 'laptop',
      createdAt: Date.now(),
    });
    render(<AgentDialog onCancel={vi.fn()} />);

    await waitFor(() => expect(screen.getByText(/no tokens yet/i)).toBeTruthy());
    await user.type(screen.getByLabelText('Label'), 'laptop');
    await user.click(screen.getByRole('button', { name: 'Create token' }));

    await waitFor(() =>
      expect(screen.getByText(mcpAddCommand(ORIGIN, 'tns_live123'))).toBeTruthy(),
    );
    expect(screen.queryByText(mcpAddCommand(ORIGIN, '<token>'))).toBeNull();
  });
});
