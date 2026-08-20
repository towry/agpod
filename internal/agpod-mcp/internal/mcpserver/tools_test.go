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

type honchoMock struct {
	mu          sync.Mutex
	messages    []honcho.Message
	conclusions []*honcho.Conclusion
}

func (m *honchoMock) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	body, _ := io.ReadAll(r.Body)
	switch {
	case r.Method == http.MethodPost && strings.HasSuffix(r.URL.Path, "/peers"):
		writeJSON(w, map[string]any{"id": "agpod-agent"})
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
	case r.Method == http.MethodPost && strings.HasSuffix(r.URL.Path, "/conclusions") && !strings.Contains(r.URL.Path, "/conclusions/"):
		m.createConclusions(w, body)
	case r.Method == http.MethodPost && strings.HasSuffix(r.URL.Path, "/conclusions/query"):
		m.queryConclusions(w)
	case r.Method == http.MethodDelete && strings.Contains(r.URL.Path, "/conclusions/"):
		w.WriteHeader(http.StatusNoContent)
	case r.Method == http.MethodPost && strings.HasSuffix(r.URL.Path, "/chat"):
		writeJSON(w, map[string]any{"content": `{"answer":"","unknown":true,"quotes":[]}`})
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

func (m *honchoMock) createConclusions(w http.ResponseWriter, body []byte) {
	var payload honcho.ConclusionBatchCreate
	_ = json.Unmarshal(body, &payload)
	out := make([]*honcho.Conclusion, 0, len(payload.Conclusions))
	m.mu.Lock()
	for i, c := range payload.Conclusions {
		conc := &honcho.Conclusion{
			ID:         "conc-" + string(rune('A'+i+len(m.conclusions))),
			Content:    c.Content,
			ObserverID: c.ObserverID,
			ObservedID: c.ObservedID,
			CreatedAt:  time.Now().UTC(),
		}
		m.conclusions = append(m.conclusions, conc)
		out = append(out, conc)
	}
	m.mu.Unlock()
	writeJSON(w, out)
}

func (m *honchoMock) queryConclusions(w http.ResponseWriter) {
	m.mu.Lock()
	out := append([]*honcho.Conclusion(nil), m.conclusions...)
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
		PeerID:    "agpod-agent",
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
	for _, name := range []string{"note", "ask_note", "forget"} {
		if !got[name] {
			t.Fatalf("expected tool %s registered, got %v", name, got)
		}
	}
	for _, name := range []string{"memo_write_finding", "remember", "memo_recall"} {
		if got[name] {
			t.Fatalf("old tool %s must not be registered", name)
		}
	}
}

func TestNoteThenFind(t *testing.T) {
	cs, ctx := newTestStack(t)
	callTool(t, cs, ctx, "note", map[string]any{
		"content": "orb login shell 不 source /etc/bashrc",
		"cues":    []string{"login shell 没有 nix"},
	})
	res := callTool(t, cs, ctx, "ask_note", map[string]any{"query": "login shell 没有 nix"})
	out := contentText(res)
	if !strings.Contains(out, "orb login shell") {
		t.Fatalf("ask_note did not surface note, got: %s", out)
	}
	if strings.Contains(out, "find:") {
		t.Fatalf("ask_note output leaked appendix: %s", out)
	}
}

func TestNoteMissingContentIsError(t *testing.T) {
	cs, ctx := newTestStack(t)
	res, err := cs.CallTool(ctx, &mcp.CallToolParams{
		Name:      "note",
		Arguments: map[string]any{"content": "", "cues": []string{"x"}},
	})
	if err != nil {
		t.Fatalf("call: %v", err)
	}
	if !res.IsError {
		t.Fatalf("expected IsError=true when content missing")
	}
}

func TestNoteMissingCuesIsError(t *testing.T) {
	cs, ctx := newTestStack(t)
	res, err := cs.CallTool(ctx, &mcp.CallToolParams{
		Name:      "note",
		Arguments: map[string]any{"content": "a standalone fact"},
	})
	if err != nil {
		t.Fatalf("call: %v", err)
	}
	if !res.IsError {
		t.Fatalf("expected IsError=true when cues missing")
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
	for _, name := range []string{"note", "forget"} {
		if got[name] {
			t.Fatalf("readonly server must not expose %s", name)
		}
	}
	if !got["ask_note"] {
		t.Fatalf("readonly server must still expose ask_note")
	}

	_, err := cs.CallTool(ctx, &mcp.CallToolParams{
		Name:      "note",
		Arguments: map[string]any{"content": "x"},
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
