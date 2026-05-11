// Package mcpserver wires the agent-memo store onto the MCP go-sdk.
package mcpserver

import (
	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/towry/agpod/internal/agpod-mcp/internal/buildinfo"
	"github.com/towry/agpod/internal/agpod-mcp/internal/memo"
)

// Options tunes server registration.
type Options struct {
	// Readonly skips registering the four mutating tools (memo_write_* and
	// memo_set_status). Read tools are always exposed.
	Readonly bool
}

// New constructs a registered MCP server. The caller drives the transport.
func New(store *memo.Store, opts Options) *mcp.Server {
	info := buildinfo.Current()
	name := "agent-memo"
	if opts.Readonly {
		name = "agent-memo-readonly"
	}
	server := mcp.NewServer(&mcp.Implementation{
		Name:    name,
		Version: info.Version,
	}, nil)
	registerTools(server, store, opts.Readonly)
	return server
}
