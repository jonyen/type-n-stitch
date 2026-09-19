// Helpers for the "Connect an agent" dialog: the copy-pasteable CLI command
// that wires Claude up to this project's MCP endpoint.

/**
 * The exact `claude mcp add` invocation for a freshly minted token. `origin`
 * is `window.location.origin` (a trailing slash, if any, is stripped so the
 * URL doesn't end up with a double slash).
 */
export function mcpAddCommand(origin: string, token: string): string {
  const base = origin.replace(/\/+$/, '');
  return `claude mcp add --transport http type-n-stitch ${base}/mcp --header "Authorization: Bearer ${token}"`;
}
