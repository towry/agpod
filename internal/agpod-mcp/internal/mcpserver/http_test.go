package mcpserver

import (
	"context"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"

	honcho "github.com/hekmon/go-honcho"
	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/towry/agpod/internal/agpod-mcp/internal/memo"
)

func TestHTTPUnauthorizedWithoutToken(t *testing.T) {
	srv := httptest.NewServer(withBearer("secret", http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusOK)
	})))
	t.Cleanup(srv.Close)
	res, err := http.Get(srv.URL)
	if err != nil {
		t.Fatal(err)
	}
	defer res.Body.Close()
	if res.StatusCode != http.StatusUnauthorized {
		t.Fatalf("status want 401, got %d", res.StatusCode)
	}
}

func TestHTTPAskNoteOverStreamable(t *testing.T) {
	honchoSrv := httptest.NewServer(&honchoMock{})
	t.Cleanup(honchoSrv.Close)
	base, _ := url.Parse(honchoSrv.URL)
	cli := honcho.New(&honcho.Options{APIKey: "test", BaseURL: base})
	store, err := memo.NewStore(cli, memo.Options{
		Workspace: "ws",
		PeerID:    "agpod-agent",
		RepoID:    "repo",
		RepoLabel: "github.com/example/repo",
	})
	if err != nil {
		t.Fatal(err)
	}
	mcpServer := New(store, Options{})
	handler := mcp.NewStreamableHTTPHandler(func(*http.Request) *mcp.Server {
		return mcpServer
	}, &mcp.StreamableHTTPOptions{JSONResponse: true, Logger: slog.New(slog.NewTextHandler(io.Discard, nil))})
	httpSrv := httptest.NewServer(handler)
	t.Cleanup(httpSrv.Close)

	ctx, cancel := context.WithTimeout(context.Background(), 8*time.Second)
	t.Cleanup(cancel)
	client := mcp.NewClient(&mcp.Implementation{Name: "test", Version: "v0"}, nil)
	cs, err := client.Connect(ctx, &mcp.StreamableClientTransport{Endpoint: httpSrv.URL}, nil)
	if err != nil {
		t.Fatalf("connect: %v", err)
	}
	t.Cleanup(func() { _ = cs.Close() })

	_, err = store.Note(ctx, memo.NoteInput{
		Content: "orb login shell 不 source /etc/bashrc",
		Cues:    []string{"login shell 没有 nix"},
	})
	if err != nil {
		t.Fatal(err)
	}
	res, err := cs.CallTool(ctx, &mcp.CallToolParams{
		Name:      "ask_note",
		Arguments: map[string]any{"query": "login shell 没有 nix"},
	})
	if err != nil {
		t.Fatalf("ask_note: %v", err)
	}
	if res.IsError {
		t.Fatalf("ask_note error: %+v", res.Content)
	}
	out := ""
	for _, c := range res.Content {
		if tc, ok := c.(*mcp.TextContent); ok {
			out += tc.Text
		}
	}
	if !strings.Contains(out, "bashrc") {
		t.Fatalf("http ask_note missed note: %s", out)
	}
}
