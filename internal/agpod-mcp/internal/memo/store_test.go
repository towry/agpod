package memo

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync"
	"testing"
	"time"

	honcho "github.com/hekmon/go-honcho"
)

type recordedRequest struct {
	method string
	path   string
	body   string
}

type honchoMock struct {
	t  *testing.T
	mu sync.Mutex

	requests []recordedRequest
	messages []honcho.Message // ordered oldest → newest

	// nextSearchResult lets a test stage the next session/workspace search.
	nextSearchResult []honcho.Message
}

func newHonchoMock(t *testing.T) (*honchoMock, *honcho.Client) {
	m := &honchoMock{t: t}
	srv := httptest.NewServer(m)
	t.Cleanup(srv.Close)
	base, err := url.Parse(srv.URL)
	if err != nil {
		t.Fatalf("parse mock url: %v", err)
	}
	cli := honcho.New(&honcho.Options{APIKey: "test", BaseURL: base})
	return m, cli
}

func (m *honchoMock) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	body, _ := io.ReadAll(r.Body)
	m.mu.Lock()
	m.requests = append(m.requests, recordedRequest{method: r.Method, path: r.URL.Path, body: string(body)})
	m.mu.Unlock()

	switch {
	case r.Method == http.MethodPost && strings.HasSuffix(r.URL.Path, "/peers"):
		writeJSON(w, map[string]any{"id": "agpod-memo"})
	case r.Method == http.MethodPost && strings.HasSuffix(r.URL.Path, "/sessions") && !strings.Contains(r.URL.Path, "/sessions/"):
		writeJSON(w, map[string]any{"id": "memo_repo"})
	case r.Method == http.MethodPost && strings.HasSuffix(r.URL.Path, "/messages"):
		m.handleCreateMessages(w, body)
	case r.Method == http.MethodPost && strings.HasSuffix(r.URL.Path, "/messages/list"):
		m.handleListMessages(w, body, r.URL.Query().Get("reverse") == "true")
	case r.Method == http.MethodPost && strings.HasSuffix(r.URL.Path, "/search"):
		m.handleSearch(w)
	case r.Method == http.MethodPut && strings.Contains(r.URL.Path, "/messages/"):
		m.handleUpdateMessage(w, r.URL.Path, body)
	default:
		http.Error(w, "unhandled route: "+r.URL.Path, http.StatusNotFound)
	}
}

func (m *honchoMock) handleCreateMessages(w http.ResponseWriter, body []byte) {
	var payload struct {
		Messages []honcho.MessageCreate `json:"messages"`
	}
	if err := json.Unmarshal(body, &payload); err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}
	out := make([]honcho.Message, 0, len(payload.Messages))
	m.mu.Lock()
	for i, mc := range payload.Messages {
		created := time.Now().UTC()
		if mc.CreatedAt != nil {
			created = *mc.CreatedAt
		}
		msg := honcho.Message{
			ID:        "hmsg-" + idFromMeta(mc.Metadata, i),
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

func (m *honchoMock) handleListMessages(w http.ResponseWriter, _ []byte, reverse bool) {
	m.mu.Lock()
	defer m.mu.Unlock()
	items := append([]honcho.Message(nil), m.messages...)
	if reverse {
		for i, j := 0, len(items)-1; i < j; i, j = i+1, j-1 {
			items[i], items[j] = items[j], items[i]
		}
	}
	writeJSON(w, honcho.PageMessage{
		Items: items,
		Total: len(items),
		Page:  1,
		Size:  len(items),
		Pages: 1,
	})
}

func (m *honchoMock) handleSearch(w http.ResponseWriter) {
	m.mu.Lock()
	out := m.nextSearchResult
	if out == nil {
		out = append([]honcho.Message(nil), m.messages...)
	}
	m.nextSearchResult = nil
	m.mu.Unlock()
	writeJSON(w, out)
}

func (m *honchoMock) handleUpdateMessage(w http.ResponseWriter, path string, body []byte) {
	id := path[strings.LastIndex(path, "/")+1:]
	var req honcho.MessageUpdate
	if err := json.Unmarshal(body, &req); err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}
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

func idFromMeta(raw json.RawMessage, fallback int) string {
	var m map[string]any
	if err := json.Unmarshal(raw, &m); err == nil {
		if v, ok := m["entry_id"].(string); ok && v != "" {
			return v
		}
	}
	return time.Now().UTC().Format("150405.000000") + "-" + string(rune('a'+fallback))
}

func writeJSON(w http.ResponseWriter, v any) {
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(v)
}

// --- tests ---

func newTestStore(t *testing.T, cli *honcho.Client) *Store {
	t.Helper()
	idGen := newSeqIDGen()
	now := newSeqClock()
	store, err := NewStore(cli, Options{
		Workspace: "ws",
		PeerID:    "agpod-memo",
		RepoID:    "repo",
		RepoLabel: "github.com/example/repo",
		Now:       now,
		ID:        idGen,
	})
	if err != nil {
		t.Fatalf("new store: %v", err)
	}
	return store
}

func newSeqIDGen() func() string {
	var i int
	return func() string {
		i++
		return "entry-" + string(rune('A'+i-1))
	}
}

func newSeqClock() func() time.Time {
	base := time.Date(2026, 5, 11, 12, 0, 0, 0, time.UTC)
	var i int
	return func() time.Time {
		i++
		return base.Add(time.Duration(i) * time.Minute)
	}
}

func TestWriteFindingPersistsMetadata(t *testing.T) {
	mock, cli := newHonchoMock(t)
	store := newTestStore(t, cli)
	ctx := context.Background()

	id, err := store.WriteFinding(ctx, WriteFindingInput{
		Content: "hooks queue is per-case",
		Scope:   []string{"crates/agpod-case/src/hooks.rs", "case-hooks"},
	})
	if err != nil {
		t.Fatalf("write finding: %v", err)
	}
	if id == "" {
		t.Fatalf("expected entry id, got empty")
	}

	mock.mu.Lock()
	defer mock.mu.Unlock()
	if len(mock.messages) != 1 {
		t.Fatalf("expected 1 stored message, got %d", len(mock.messages))
	}
	var meta map[string]any
	if err := json.Unmarshal(mock.messages[0].Metadata, &meta); err != nil {
		t.Fatalf("decode metadata: %v", err)
	}
	if meta["entry_type"] != string(EntryFinding) {
		t.Fatalf("entry_type want finding, got %v", meta["entry_type"])
	}
	if meta["status"] != string(StatusLive) {
		t.Fatalf("status want live, got %v", meta["status"])
	}
	if _, ok := meta["scope"]; !ok {
		t.Fatalf("scope missing from metadata")
	}
	if _, ok := meta["evidence_refs"]; ok {
		t.Fatalf("evidence_refs should be omitted when empty")
	}
}

func TestWriteDecisionSupersedeMarksOld(t *testing.T) {
	mock, cli := newHonchoMock(t)
	store := newTestStore(t, cli)
	ctx := context.Background()

	oldID, err := store.WriteDecision(ctx, WriteDecisionInput{
		Content: "use surrealdb embedded with rocksdb",
		Scope:   []string{"crates/agpod-case/src/client.rs"},
	})
	if err != nil {
		t.Fatalf("first decision: %v", err)
	}

	newID, err := store.WriteDecision(ctx, WriteDecisionInput{
		Content:         "switch to surrealdb mem backend for tests",
		Scope:           []string{"crates/agpod-case/src/client.rs"},
		Supersedes:      oldID,
		SupersedeReason: "rocksdb hangs on parallel tests",
	})
	if err != nil {
		t.Fatalf("supersede decision: %v", err)
	}
	if newID == oldID {
		t.Fatalf("new id must differ from old")
	}

	mock.mu.Lock()
	var oldMeta map[string]any
	for _, msg := range mock.messages {
		var m map[string]any
		_ = json.Unmarshal(msg.Metadata, &m)
		if m["entry_id"] == oldID {
			oldMeta = m
			break
		}
	}
	mock.mu.Unlock()
	if oldMeta == nil {
		t.Fatalf("old decision not found in mock store")
	}
	if oldMeta["status"] != string(StatusSuperseded) {
		t.Fatalf("expected old status superseded, got %v", oldMeta["status"])
	}
}

func TestPickupHandoffLatest(t *testing.T) {
	_, cli := newHonchoMock(t)
	store := newTestStore(t, cli)
	ctx := context.Background()

	_, _ = store.WriteHandoff(ctx, WriteHandoffInput{Summary: "first", Content: "older snapshot"})
	wantID, err := store.WriteHandoff(ctx, WriteHandoffInput{Summary: "latest", Content: "newest snapshot"})
	if err != nil {
		t.Fatalf("second handoff: %v", err)
	}

	res, err := store.PickupHandoff(ctx, PickupHandoffInput{})
	if err != nil {
		t.Fatalf("pickup: %v", err)
	}
	if res.EntryID != wantID {
		t.Fatalf("latest pickup want %s, got %s", wantID, res.EntryID)
	}
	if res.Summary != "latest" {
		t.Fatalf("summary want latest, got %q", res.Summary)
	}
}

func TestPickupHandoffByID(t *testing.T) {
	_, cli := newHonchoMock(t)
	store := newTestStore(t, cli)
	ctx := context.Background()

	wantID, _ := store.WriteHandoff(ctx, WriteHandoffInput{Summary: "first", Content: "older"})
	_, _ = store.WriteHandoff(ctx, WriteHandoffInput{Summary: "second", Content: "newer"})

	res, err := store.PickupHandoff(ctx, PickupHandoffInput{HandoffID: wantID})
	if err != nil {
		t.Fatalf("pickup by id: %v", err)
	}
	if res.EntryID != wantID {
		t.Fatalf("want %s, got %s", wantID, res.EntryID)
	}
}

func TestRecallFiltersHandoffsByDefault(t *testing.T) {
	_, cli := newHonchoMock(t)
	store := newTestStore(t, cli)
	ctx := context.Background()

	_, _ = store.WriteFinding(ctx, WriteFindingInput{Content: "f1", Scope: []string{"file.rs:1"}})
	_, _ = store.WriteHandoff(ctx, WriteHandoffInput{Summary: "h1", Content: "narrative"})

	hits, err := store.Recall(ctx, RecallInput{})
	if err != nil {
		t.Fatalf("recall: %v", err)
	}
	for _, h := range hits {
		if h.EntryType == EntryHandoff {
			t.Fatalf("default recall must skip handoffs")
		}
	}

	hits, err = store.Recall(ctx, RecallInput{IncludeHandoff: true})
	if err != nil {
		t.Fatalf("recall with handoff: %v", err)
	}
	var sawHandoff bool
	for _, h := range hits {
		if h.EntryType == EntryHandoff {
			sawHandoff = true
		}
	}
	if !sawHandoff {
		t.Fatalf("include_handoff=true should surface handoffs")
	}
}

func TestWhyReturnsLiveDecisionsWithChain(t *testing.T) {
	_, cli := newHonchoMock(t)
	store := newTestStore(t, cli)
	ctx := context.Background()

	old, _ := store.WriteDecision(ctx, WriteDecisionInput{
		Content: "rocksdb",
		Scope:   []string{"db-backend"},
	})
	newID, err := store.WriteDecision(ctx, WriteDecisionInput{
		Content:         "mem",
		Scope:           []string{"db-backend"},
		Supersedes:      old,
		SupersedeReason: "tests hang",
	})
	if err != nil {
		t.Fatalf("supersede: %v", err)
	}

	res, err := store.Why(ctx, WhyInput{Scope: "db-backend"})
	if err != nil {
		t.Fatalf("why: %v", err)
	}
	if len(res.Decisions) != 1 {
		t.Fatalf("expected 1 live decision, got %d", len(res.Decisions))
	}
	dv := res.Decisions[0]
	if dv.EntryID != newID {
		t.Fatalf("live decision want %s, got %s", newID, dv.EntryID)
	}
	if len(dv.SupersedesChain) != 1 || dv.SupersedesChain[0].EntryID != old {
		t.Fatalf("supersedes chain unexpected: %+v", dv.SupersedesChain)
	}
	if dv.SupersedesChain[0].SupersedeReason != "tests hang" {
		t.Fatalf("supersede reason missing")
	}
}

func TestRecallExcludesSupersededEntries(t *testing.T) {
	_, cli := newHonchoMock(t)
	store := newTestStore(t, cli)
	ctx := context.Background()

	oldID, _ := store.WriteDecision(ctx, WriteDecisionInput{
		Content: "old path",
		Scope:   []string{"x"},
	})
	_, err := store.WriteDecision(ctx, WriteDecisionInput{
		Content:    "new path",
		Scope:      []string{"x"},
		Supersedes: oldID,
	})
	if err != nil {
		t.Fatalf("supersede: %v", err)
	}

	hits, err := store.Recall(ctx, RecallInput{})
	if err != nil {
		t.Fatalf("recall: %v", err)
	}
	for _, h := range hits {
		if h.EntryID == oldID {
			t.Fatalf("superseded entry should not appear in recall; got status=%s", h.Status)
		}
	}
}

func TestRecallRejectsCrossRepoWithoutQuery(t *testing.T) {
	_, cli := newHonchoMock(t)
	store := newTestStore(t, cli)
	_, err := store.Recall(context.Background(), RecallInput{CrossRepo: true})
	if !errors.Is(err, ErrCrossRepoRequiresQuery) {
		t.Fatalf("want ErrCrossRepoRequiresQuery, got %v", err)
	}
}

func TestSetStatusRejectsLive(t *testing.T) {
	_, cli := newHonchoMock(t)
	store := newTestStore(t, cli)
	if err := store.SetStatus(context.Background(), SetStatusInput{EntryID: "x", Status: StatusLive}); err == nil {
		t.Fatalf("expected error for setting status to live")
	}
}
