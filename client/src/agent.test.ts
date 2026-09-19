import { describe, expect, it } from 'vitest';

import { mcpAddCommand } from './agent';

describe('mcpAddCommand', () => {
  it('builds the exact claude mcp add invocation', () => {
    expect(mcpAddCommand('http://localhost:5174', 'tns_abc')).toBe(
      'claude mcp add --transport http type-n-stitch http://localhost:5174/mcp --header "Authorization: Bearer tns_abc"',
    );
  });

  it('strips a trailing slash from the origin', () => {
    expect(mcpAddCommand('http://localhost:5174/', 'tns_abc')).toBe(
      'claude mcp add --transport http type-n-stitch http://localhost:5174/mcp --header "Authorization: Bearer tns_abc"',
    );
  });

  it('strips multiple trailing slashes', () => {
    expect(mcpAddCommand('https://example.com//', 'tns_xyz')).toBe(
      'claude mcp add --transport http type-n-stitch https://example.com/mcp --header "Authorization: Bearer tns_xyz"',
    );
  });
});
