// Helpers for the "Connect AI" dialog: the MCP endpoint URL and the
// copy-pasteable setup snippets for the various ways a client can be pointed
// at it.

/** Strip a trailing slash (or several) so joining a path never doubles up. */
function stripTrailingSlash(origin: string): string {
  return origin.replace(/\/+$/, '');
}

/** This project's MCP endpoint. `origin` is `window.location.origin`. */
export function mcpEndpoint(origin: string): string {
  return `${stripTrailingSlash(origin)}/mcp`;
}

/** The `Authorization` header value a client sends the token with. */
export function authHeader(token: string): string {
  return `Authorization: Bearer ${token}`;
}

/**
 * The exact `claude mcp add` invocation for a freshly minted token. `origin`
 * is `window.location.origin` (a trailing slash, if any, is stripped so the
 * URL doesn't end up with a double slash).
 */
export function mcpAddCommand(origin: string, token: string): string {
  return `claude mcp add --transport http type-n-stitch ${mcpEndpoint(origin)} --header "${authHeader(token)}"`;
}

/** The JSON config block for HTTP-aware clients (Cursor, VS Code, Windsurf). */
export function mcpJsonConfig(origin: string, token: string): string {
  return JSON.stringify(
    {
      mcpServers: {
        'type-n-stitch': {
          type: 'http',
          url: mcpEndpoint(origin),
          headers: { Authorization: `Bearer ${token}` },
        },
      },
    },
    null,
    2,
  );
}

/**
 * The JSON config block for stdio-only clients (Claude Desktop), which reach
 * the streamable-HTTP endpoint through `mcp-remote`.
 */
export function mcpRemoteConfig(origin: string, token: string): string {
  return JSON.stringify(
    {
      mcpServers: {
        'type-n-stitch': {
          command: 'npx',
          args: ['-y', 'mcp-remote', mcpEndpoint(origin), '--header', authHeader(token)],
        },
      },
    },
    null,
    2,
  );
}
