package mcpserver

import (
	"context"
	"log/slog"
	"net"
	"net/http"
	"strings"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// ListenAndServe serves the MCP server over Streamable HTTP until ctx is cancelled.
func ListenAndServe(ctx context.Context, server *mcp.Server, addr, token string, logger *slog.Logger) error {
	handler := mcp.NewStreamableHTTPHandler(func(*http.Request) *mcp.Server {
		return server
	}, &mcp.StreamableHTTPOptions{
		Logger:         logger,
		JSONResponse:   true,
		SessionTimeout: 30 * time.Minute,
	})
	mux := http.NewServeMux()
	mux.Handle("/mcp", withBearer(token, handler))
	mux.HandleFunc("/healthz", func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte("ok\n"))
	})

	httpServer := &http.Server{
		Addr:              addr,
		Handler:           mux,
		ReadHeaderTimeout: 10 * time.Second,
		BaseContext:       func(net.Listener) context.Context { return ctx },
	}

	errCh := make(chan error, 1)
	go func() {
		logger.Info("agent-memo http listen", "addr", addr, "path", "/mcp")
		errCh <- httpServer.ListenAndServe()
	}()

	select {
	case <-ctx.Done():
		shutCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_ = httpServer.Shutdown(shutCtx)
		return ctx.Err()
	case err := <-errCh:
		if err == http.ErrServerClosed {
			return nil
		}
		return err
	}
}

func withBearer(token string, next http.Handler) http.Handler {
	if token == "" {
		return next
	}
	want := "Bearer " + token
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		got := strings.TrimSpace(r.Header.Get("Authorization"))
		if got != want {
			w.Header().Set("WWW-Authenticate", `Bearer realm="agpod-mcp"`)
			http.Error(w, "unauthorized", http.StatusUnauthorized)
			return
		}
		next.ServeHTTP(w, r)
	})
}
