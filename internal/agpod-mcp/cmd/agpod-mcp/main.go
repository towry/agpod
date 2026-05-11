// Command agpod-mcp serves the agent-memo MCP tools over stdio.
// The Go binary supersedes the Rust crate at crates/agpod-mcp/ (which is on
// a deprecation path). The surrounding Go module can host additional
// commands later.
package main

import (
	"context"
	"fmt"
	"log/slog"
	"os"
	"os/signal"
	"syscall"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/towry/agpod/internal/agpod-mcp/internal/buildinfo"
	"github.com/towry/agpod/internal/agpod-mcp/internal/config"
	"github.com/towry/agpod/internal/agpod-mcp/internal/logging"
	"github.com/towry/agpod/internal/agpod-mcp/internal/mcpserver"
	"github.com/towry/agpod/internal/agpod-mcp/internal/memo"
	"github.com/towry/agpod/internal/agpod-mcp/internal/repoid"
)

func main() {
	if err := run(); err != nil {
		fmt.Fprintf(os.Stderr, "agpod-mcp: %v\n", err)
		os.Exit(1)
	}
}

func run() error {
	logger, err := logging.ConfigureDefault(os.Stderr)
	if err != nil {
		return err
	}
	info := buildinfo.Current()
	logger.Info("agent-memo starting", "version", info.Version, "commit", info.Commit)

	cfg, err := config.FromEnv()
	if err != nil {
		return fmt.Errorf("load config: %w", err)
	}

	identity, err := repoid.Resolve(cfg.RepoRoot)
	if err != nil {
		return fmt.Errorf("resolve repo identity (root=%s): %w", cfg.RepoRoot, err)
	}
	logger.Info("repo identity resolved",
		"repo_id", identity.RepoID,
		"repo_label", identity.RepoLabel,
		"readonly", cfg.Readonly)

	cli, err := memo.NewClient(cfg)
	if err != nil {
		return err
	}
	store, err := memo.NewStore(cli, memo.Options{
		Workspace: cfg.HonchoWorkspaceID,
		PeerID:    cfg.PeerID,
		RepoID:    identity.RepoID,
		RepoLabel: identity.RepoLabel,
	})
	if err != nil {
		return err
	}

	ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer cancel()

	if err := store.Ensure(ctx); err != nil {
		// Log but continue: on Honcho outage tools will surface the error per-call.
		slog.Warn("ensure peer/session failed; tools will retry on first call", "err", err)
	}

	server := mcpserver.New(store, mcpserver.Options{Readonly: cfg.Readonly})
	if err := server.Run(ctx, &mcp.StdioTransport{}); err != nil {
		return fmt.Errorf("mcp server: %w", err)
	}
	return nil
}
