import { describe, expect, it } from 'vitest';

import { authHeader, mcpAddCommand, mcpEndpoint, mcpJsonConfig, mcpRemoteConfig } from './agent';

describe('mcpEndpoint', () => {
  it('appends /mcp to the origin', () => {
    expect(mcpEndpoint('http://localhost:5174')).toBe('http://localhost:5174/mcp');
  });

  it('strips a trailing slash from the origin', () => {
    expect(mcpEndpoint('http://localhost:5174/')).toBe('http://localhost:5174/mcp');
  });

  it('strips multiple trailing slashes', () => {
    expect(mcpEndpoint('https://example.com//')).toBe('https://example.com/mcp');
  });
});

describe('authHeader', () => {
  it('builds the bearer header value', () => {
    expect(authHeader('tns_abc')).toBe('Authorization: Bearer tns_abc');
  });
});

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

describe('mcpJsonConfig', () => {
  it('parses to the exact HTTP client config shape', () => {
    const json = mcpJsonConfig('http://localhost:5174', 'tns_abc');
    expect(JSON.parse(json)).toEqual({
      mcpServers: {
        'type-n-stitch': {
          type: 'http',
          url: 'http://localhost:5174/mcp',
          headers: { Authorization: 'Bearer tns_abc' },
        },
      },
    });
  });

  it('is pretty-printed with 2-space indentation', () => {
    const json = mcpJsonConfig('http://localhost:5174', 'tns_abc');
    expect(json).toContain('\n  "mcpServers"');
  });

  it('strips a trailing slash from the origin', () => {
    const json = mcpJsonConfig('http://localhost:5174/', 'tns_abc');
    expect(JSON.parse(json).mcpServers['type-n-stitch'].url).toBe('http://localhost:5174/mcp');
  });
});

describe('mcpRemoteConfig', () => {
  it('parses to the exact mcp-remote stdio config shape', () => {
    const json = mcpRemoteConfig('http://localhost:5174', 'tns_abc');
    expect(JSON.parse(json)).toEqual({
      mcpServers: {
        'type-n-stitch': {
          command: 'npx',
          args: [
            '-y',
            'mcp-remote',
            'http://localhost:5174/mcp',
            '--header',
            'Authorization: Bearer tns_abc',
          ],
        },
      },
    });
  });

  it('is pretty-printed with 2-space indentation', () => {
    const json = mcpRemoteConfig('http://localhost:5174', 'tns_abc');
    expect(json).toContain('\n  "mcpServers"');
  });

  it('strips a trailing slash from the origin', () => {
    const json = mcpRemoteConfig('http://localhost:5174/', 'tns_abc');
    expect(JSON.parse(json).mcpServers['type-n-stitch'].args[2]).toBe('http://localhost:5174/mcp');
  });
});
