package mcpserver

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync"
	"testing"
	"time"

	honcho "github.com/hekmon/go-honcho"
	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/towry/agpod/internal/agpod-mcp/internal/memo"
)

// minimal honcho mock — mirrors the one in memo/store_test.go but lives here
// to keep the package boundary clean.
type honchoMock struct {
	mu       sync.Mutex
	messages []honcho.Message
}

func (m *honchoMock) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	body, _ := io.ReadAll(r.Body)
	switch {
	case r.Method == http.MethodPost && strings.HasSuffix(r.URL.Path, "/peers"):
		writeJSON(w, map[string]any{"id": "agpod-memo"})
	case r.Method == http.MethodPost && strings.HasSuffix(r.URL.Path, "/sessions") && !strings.Contains(r.URL.Path, "/sessions/"):
		writeJSON(w, map[string]any{"id": "memo_repo"})
	case r.Method == http.MethodPost && strings.HasSuffix(r.URL.Path, "/messages"):
		m.createMessages(w, body)
	case r.Method == http.MethodPost && strings.HasSuffix(r.URL.Path, "/messages/list"):
		m.listMessages(w, r.URL.Query().Get("reverse") == "true")
	case r.Method == http.MethodPost && strings.HasSuffix(r.URL.Path, "/search"):
		m.search(w)
	case r.Method == http.MethodPut && strings.Contains(r.URL.Path, "/messages/"):
		m.updateMessage(w, r.URL.Path, body)
	default:
		http.Error(w, "unhandled: "+r.URL.Path, http.StatusNotFound)
	}
}

func (m *honchoMock) createMessages(w http.ResponseWriter, body []byte) {
	var payload struct {
		Messages []honcho.MessageCreate `json:"messages"`
	}
	_ = json.Unmarshal(body, &payload)
	out := make([]honcho.Message, 0, len(payload.Messages))
	m.mu.Lock()
	for i, mc := range payload.Messages {
		created := time.Now().UTC()
		if mc.CreatedAt != nil {
			created = *mc.CreatedAt
		}
		msg := honcho.Message{
			ID:        "hmsg-" + entryIDFrom(mc.Metadata, i),
			Content:   mc.Content,
			PeerID:    mc.PeerID,
			SessionID: "memo_repo",
			Metadata:  mc.Metadata,
			CreatedAt: created,
		}
		m.messages = append(m.messages, msg)
		out = append(out, msg)
	}
	m.mu.Unlock()
	writeJSON(w, out)
}

func (m *honchoMock) listMessages(w http.ResponseWriter, reverse bool) {
	m.mu.Lock()
	defer m.mu.Unlock()
	items := append([]honcho.Message(nil), m.messages...)
	if reverse {
		for i, j := 0, len(items)-1; i < j; i, j = i+1, j-1 {
			items[i], items[j] = items[j], items[i]
		}
	}
	writeJSON(w, honcho.PageMessage{Items: items, Total: len(items), Page: 1, Size: len(items), Pages: 1})
}

func (m *honchoMock) search(w http.ResponseWriter) {
	m.mu.Lock()
	out := append([]honcho.Message(nil), m.messages...)
	m.mu.Unlock()
	writeJSON(w, out)
}

func (m *honchoMock) updateMessage(w http.ResponseWriter, path string, body []byte) {
	id := path[strings.LastIndex(path, "/")+1:]
	var req honcho.MessageUpdate
	_ = json.Unmarshal(body, &req)
	m.mu.Lock()
	defer m.mu.Unlock()
	for i := range m.messages {
		if m.messages[i].ID == id {
			m.messages[i].Metadata = req.Metadata
			writeJSON(w, m.messages[i])
			return
		}
	}
	http.Error(w, "not found", http.StatusNotFound)
}

func entryIDFrom(raw json.RawMessage, fallback int) string {
	var m map[string]any
	if err := json.Unmarshal(raw, &m); err == nil {
		if v, ok := m["entry_id"].(string); ok && v != "" {
			return v
		}
	}
	return time.Now().UTC().Format("150405.000000") + string(rune('a'+fallback))
}

func writeJSON(w http.ResponseWriter, v any) {
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(v)
}

// newTestStack starts the mock backend, builds a Store with deterministic ids
// and clock, registers the MCP server, and connects an in-memory client.
func newTestStack(t *testing.T) (*mcp.ClientSession, context.Context) {
	return newTestStackWithOpts(t, false)
}

func newTestStackWithOpts(t *testing.T, readonly bool) (*mcp.ClientSession, context.Context) {
	t.Helper()
	srv := httptest.NewServer(&honchoMock{})
	t.Cleanup(srv.Close)
	base, _ := url.Parse(srv.URL)
	cli := honcho.New(&honcho.Options{APIKey: "test", BaseURL: base})

	var idCounter int
	idFn := func() string { idCounter++; return "entry-" + string(rune('A'+idCounter-1)) }
	baseTime := time.Date(2026, 5, 11, 12, 0, 0, 0, time.UTC)
	var tCounter int
	nowFn := func() time.Time { tCounter++; return baseTime.Add(time.Duration(tCounter) * time.Minute) }

	store, err := memo.NewStore(cli, memo.Options{
		Workspace: "ws",
		PeerID:    "agpod-memo",
		RepoID:    "repo",
		RepoLabel: "github.com/example/repo",
		ID:        idFn,
		Now:       nowFn,
	})
	if err != nil {
		t.Fatalf("new store: %v", err)
	}

	server := New(store, Options{Readonly: readonly})
	client := mcp.NewClient(&mcp.Implementation{Name: "test", Version: "v0"}, nil)
	t1, t2 := mcp.NewInMemoryTransports()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	t.Cleanup(cancel)
	if _, err := server.Connect(ctx, t1, nil); err != nil {
		t.Fatalf("server connect: %v", err)
	}
	cs, err := client.Connect(ctx, t2, nil)
	if err != nil {
		t.Fatalf("client connect: %v", err)
	}
	t.Cleanup(func() { _ = cs.Close() })
	return cs, ctx
}

func callTool(t *testing.T, cs *mcp.ClientSession, ctx context.Context, name string, args any) *mcp.CallToolResult {
	t.Helper()
	res, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: name, Arguments: args})
	if err != nil {
		t.Fatalf("call %s: %v", name, err)
	}
	if res.IsError {
		t.Fatalf("tool %s reported error: %+v", name, res.Content)
	}
	return res
}

func TestToolsExposed(t *testing.T) {
	cs, ctx := newTestStack(t)
	got := map[string]bool{}
	for tool, err := range cs.Tools(ctx, nil) {
		if err != nil {
			t.Fatalf("list tools: %v", err)
		}
		got[tool.Name] = true
	}
	want := []string{
		"memo_write_finding", "memo_write_decision", "memo_write_handoff",
		"memo_pickup_handoff", "memo_recall", "memo_why", "memo_set_status",
	}
	for _, name := range want {
		if !got[name] {
			t.Fatalf("expected tool %s registered, got %v", name, got)
		}
	}
}

func TestWriteFindingThenRecall(t *testing.T) {
	cs, ctx := newTestStack(t)
	callTool(t, cs, ctx, "memo_write_finding", map[string]any{
		"content": "hooks queue is per-case",
		"scope":   []string{"hooks.rs", "case-hooks"},
	})
	res := callTool(t, cs, ctx, "memo_recall", map[string]any{})
	out := contentText(res)
	if !strings.Contains(out, "hooks queue is per-case") {
		t.Fatalf("recall did not surface finding, got: %s", out)
	}
}

func TestPickupHandoffRoundTrip(t *testing.T) {
	cs, ctx := newTestStack(t)
	callTool(t, cs, ctx, "memo_write_handoff", map[string]any{
		"summary": "wip on store",
		"content": "tests are green; next: tools_test",
	})
	res := callTool(t, cs, ctx, "memo_pickup_handoff", map[string]any{})
	out := contentText(res)
	if !strings.Contains(out, "wip on store") {
		t.Fatalf("pickup missing summary: %s", out)
	}
	if !strings.Contains(out, "tools_test") {
		t.Fatalf("pickup missing content: %s", out)
	}
}

func TestWriteDecisionMissingScopeIsError(t *testing.T) {
	cs, ctx := newTestStack(t)
	res, err := cs.CallTool(ctx, &mcp.CallToolParams{
		Name:      "memo_write_decision",
		Arguments: map[string]any{"content": "choose X"},
	})
	if err != nil {
		t.Fatalf("call: %v", err)
	}
	if !res.IsError {
		t.Fatalf("expected IsError=true when scope missing")
	}
}

func TestReadonlyOmitsMutatingTools(t *testing.T) {
	cs, ctx := newTestStackWithOpts(t, true)

	got := map[string]bool{}
	for tool, err := range cs.Tools(ctx, nil) {
		if err != nil {
			t.Fatalf("list tools: %v", err)
		}
		got[tool.Name] = true
	}
	for _, name := range []string{"memo_write_finding", "memo_write_decision", "memo_write_handoff", "memo_set_status"} {
		if got[name] {
			t.Fatalf("readonly server must not expose %s", name)
		}
	}
	for _, name := range []string{"memo_pickup_handoff", "memo_recall", "memo_why"} {
		if !got[name] {
			t.Fatalf("readonly server must still expose %s", name)
		}
	}

	// Calling a hidden write tool should return a protocol error, not a
	// silent success.
	_, err := cs.CallTool(ctx, &mcp.CallToolParams{
		Name:      "memo_write_finding",
		Arguments: map[string]any{"content": "x", "scope": []string{"y"}},
	})
	if err == nil {
		t.Fatalf("expected protocol error for unknown tool in readonly mode")
	}
}

func contentText(res *mcp.CallToolResult) string {
	var b strings.Builder
	for _, c := range res.Content {
		if tc, ok := c.(*mcp.TextContent); ok {
			b.WriteString(tc.Text)
		}
	}
	return b.String()
}
