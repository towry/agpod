package memo

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strconv"
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

	requests    []recordedRequest
	messages    []honcho.Message
	conclusions []*honcho.Conclusion
	nextSearch  []honcho.Message
	nextQuery   []*honcho.Conclusion
	chatContent string
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
		writeJSON(w, map[string]any{"id": "agpod-agent"})
	case r.Method == http.MethodPost && strings.HasSuffix(r.URL.Path, "/sessions") && !strings.Contains(r.URL.Path, "/sessions/"):
		writeJSON(w, map[string]any{"id": "memo_repo"})
	case r.Method == http.MethodPost && strings.HasSuffix(r.URL.Path, "/messages"):
		m.handleCreateMessages(w, body)
	case r.Method == http.MethodPost && strings.HasSuffix(r.URL.Path, "/messages/list"):
		m.handleListMessages(w, r.URL.Query().Get("reverse") == "true", r.URL.Query().Get("page"), r.URL.Query().Get("size"))
	case r.Method == http.MethodPost && strings.HasSuffix(r.URL.Path, "/search"):
		m.handleSearch(w)
	case r.Method == http.MethodPut && strings.Contains(r.URL.Path, "/messages/"):
		m.handleUpdateMessage(w, r.URL.Path, body)
	case r.Method == http.MethodPost && strings.HasSuffix(r.URL.Path, "/conclusions") && !strings.Contains(r.URL.Path, "/conclusions/"):
		m.handleCreateConclusions(w, body)
	case r.Method == http.MethodPost && strings.HasSuffix(r.URL.Path, "/conclusions/query"):
		m.handleQueryConclusions(w)
	case r.Method == http.MethodPost && strings.HasSuffix(r.URL.Path, "/conclusions/list"):
		m.handleListConclusions(w)
	case r.Method == http.MethodDelete && strings.Contains(r.URL.Path, "/conclusions/"):
		m.handleDeleteConclusion(r.URL.Path)
		w.WriteHeader(http.StatusNoContent)
	case r.Method == http.MethodPost && strings.HasSuffix(r.URL.Path, "/chat"):
		content := m.chatContent
		writeJSON(w, map[string]any{"content": content})
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

func (m *honchoMock) handleListMessages(w http.ResponseWriter, reverse bool, pageStr, sizeStr string) {
	m.mu.Lock()
	defer m.mu.Unlock()
	items := append([]honcho.Message(nil), m.messages...)
	if reverse {
		for i, j := 0, len(items)-1; i < j; i, j = i+1, j-1 {
			items[i], items[j] = items[j], items[i]
		}
	}
	size := len(items)
	if sizeStr != "" {
		if n, err := strconv.Atoi(sizeStr); err == nil && n > 0 {
			size = n
		}
	}
	page := 1
	if pageStr != "" {
		if n, err := strconv.Atoi(pageStr); err == nil && n > 0 {
			page = n
		}
	}
	total := len(items)
	pages := 1
	if size > 0 {
		pages = (total + size - 1) / size
		if pages == 0 {
			pages = 1
		}
	}
	start := (page - 1) * size
	if start > total {
		start = total
	}
	end := start + size
	if end > total {
		end = total
	}
	writeJSON(w, honcho.PageMessage{Items: items[start:end], Total: total, Page: page, Size: size, Pages: pages})
}

func (m *honchoMock) handleSearch(w http.ResponseWriter) {
	m.mu.Lock()
	out := m.nextSearch
	if out == nil {
		out = append([]honcho.Message(nil), m.messages...)
	}
	m.nextSearch = nil
	m.mu.Unlock()
	writeJSON(w, out)
}

func (m *honchoMock) handleCreateConclusions(w http.ResponseWriter, body []byte) {
	var payload honcho.ConclusionBatchCreate
	if err := json.Unmarshal(body, &payload); err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}
	out := make([]*honcho.Conclusion, 0, len(payload.Conclusions))
	m.mu.Lock()
	for i, c := range payload.Conclusions {
		id := "conc-" + string(rune('A'+i+len(m.conclusions)))
		sid := ""
		if c.SessionID != nil {
			sid = *c.SessionID
		}
		conc := &honcho.Conclusion{
			ID:         id,
			Content:    c.Content,
			ObserverID: c.ObserverID,
			ObservedID: c.ObservedID,
			CreatedAt:  time.Now().UTC(),
		}
		if sid != "" {
			conc.SessionID = &sid
		}
		m.conclusions = append(m.conclusions, conc)
		out = append(out, conc)
	}
	m.mu.Unlock()
	writeJSON(w, out)
}

func (m *honchoMock) handleListConclusions(w http.ResponseWriter) {
	m.mu.Lock()
	items := make([]honcho.Conclusion, 0, len(m.conclusions))
	for _, c := range m.conclusions {
		if c != nil {
			items = append(items, *c)
		}
	}
	m.mu.Unlock()
	writeJSON(w, honcho.PageConclusion{Items: items, Total: len(items), Page: 1, Size: len(items), Pages: 1})
}

func (m *honchoMock) handleDeleteConclusion(path string) {
	id := path[strings.LastIndex(path, "/")+1:]
	m.mu.Lock()
	defer m.mu.Unlock()
	out := m.conclusions[:0]
	for _, c := range m.conclusions {
		if c != nil && c.ID != id {
			out = append(out, c)
		}
	}
	m.conclusions = out
}

func (m *honchoMock) handleQueryConclusions(w http.ResponseWriter) {
	m.mu.Lock()
	out := m.nextQuery
	if out == nil {
		out = append([]*honcho.Conclusion(nil), m.conclusions...)
	}
	m.nextQuery = nil
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

func newTestStore(t *testing.T, cli *honcho.Client) *Store {
	t.Helper()
	store, err := NewStore(cli, Options{
		Workspace: "ws",
		PeerID:    "agpod-agent",
		RepoID:    "repo",
		RepoLabel: "github.com/example/repo",
		Now:       newSeqClock(),
		ID:        newSeqIDGen(),
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

func TestNotePersistsMessageAndConclusion(t *testing.T) {
	mock, cli := newHonchoMock(t)
	store := newTestStore(t, cli)
	ctx := context.Background()

	res, err := store.Note(ctx, NoteInput{
		Content: "orb login shell 不 source /etc/bashrc，Determinate Nix 不在 PATH；agpod 用 ~/.config/agpod/nix-profile.sh 补。",
		Cues:    []string{"login shell 没有 nix"},
	})
	if err != nil {
		t.Fatalf("note: %v", err)
	}
	if res.ID != "entry-A" {
		t.Fatalf("id want entry-A, got %s", res.ID)
	}
	if !res.Indexed {
		t.Fatalf("expected indexed=true")
	}

	mock.mu.Lock()
	defer mock.mu.Unlock()
	if len(mock.messages) != 1 {
		t.Fatalf("expected 1 message, got %d", len(mock.messages))
	}
	if !strings.Contains(mock.messages[0].Content, "find: ") {
		t.Fatalf("message body should include find appendix, got %q", mock.messages[0].Content)
	}
	if !strings.Contains(mock.messages[0].Content, "login shell 没有 nix") {
		t.Fatalf("appendix missing cue")
	}
	var meta map[string]any
	if err := json.Unmarshal(mock.messages[0].Metadata, &meta); err != nil {
		t.Fatalf("meta: %v", err)
	}
	if meta["schema"] != schemaV1 {
		t.Fatalf("schema: %v", meta["schema"])
	}
	if meta["status"] != statusLive {
		t.Fatalf("status: %v", meta["status"])
	}
	if meta["conclusion_id"] == nil || meta["conclusion_id"] == "" {
		t.Fatalf("conclusion_id not patched back: %v", meta)
	}
	if len(mock.conclusions) != 1 {
		t.Fatalf("expected 1 conclusion, got %d", len(mock.conclusions))
	}
	if strings.Contains(mock.conclusions[0].Content, "find:") {
		t.Fatalf("conclusion must stay clean, got %q", mock.conclusions[0].Content)
	}
}

func TestNoteRequiresContent(t *testing.T) {
	_, cli := newHonchoMock(t)
	store := newTestStore(t, cli)
	if _, err := store.Note(context.Background(), NoteInput{Content: "  ", Cues: []string{"x"}}); err == nil {
		t.Fatalf("expected error")
	}
}

func TestNoteRequiresCues(t *testing.T) {
	_, cli := newHonchoMock(t)
	store := newTestStore(t, cli)
	if _, err := store.Note(context.Background(), NoteInput{Content: "a standalone fact"}); err == nil {
		t.Fatalf("expected error when cues missing")
	}
}

func TestFindSearchMergesAndRanksCueOverlap(t *testing.T) {
	mock, cli := newHonchoMock(t)
	store := newTestStore(t, cli)
	ctx := context.Background()

	nix, err := store.Note(ctx, NoteInput{
		Content: "orb login shell 不 source /etc/bashrc，Determinate Nix 不在 PATH。",
		Cues:    []string{"login shell 没有 nix"},
	})
	if err != nil {
		t.Fatalf("note nix: %v", err)
	}
	_, err = store.Note(ctx, NoteInput{
		Content: "Honcho message metadata must be flat; drop empty arrays before send.",
		Cues:    []string{"honcho metadata 扁平"},
	})
	if err != nil {
		t.Fatalf("note db: %v", err)
	}

	// Search returns both; cue overlap should put nix first for this query.
	fr, err := store.search(ctx, "login shell 没有 nix", 8)
	if err != nil {
		t.Fatalf("search: %v", err)
	}
	if fr.Status != "ok" {
		t.Fatalf("status: %s", fr.Status)
	}
	if len(fr.Hits) == 0 {
		t.Fatalf("no hits")
	}
	if fr.Hits[0].ID != nix.ID {
		t.Fatalf("hit@1 want %s, got %+v", nix.ID, fr.Hits)
	}
	if strings.Contains(fr.Hits[0].Content, "find:") {
		t.Fatalf("hit content must be clean")
	}
	_ = mock
}

func TestFindAskUnknown(t *testing.T) {
	mock, cli := newHonchoMock(t)
	mock.chatContent = `{"answer":"","unknown":true,"quotes":[]}`
	store := newTestStore(t, cli)
	ar, err := store.Find(context.Background(), FindInput{Query: "user's favorite color"})
	if err != nil {
		t.Fatalf("find: %v", err)
	}
	if !ar.Unknown {
		t.Fatalf("want unknown")
	}
	if ar.Answer != "" {
		t.Fatalf("answer should be empty: %q", ar.Answer)
	}
}

func TestForgetRetiresMessage(t *testing.T) {
	mock, cli := newHonchoMock(t)
	store := newTestStore(t, cli)
	ctx := context.Background()
	res, err := store.Note(ctx, NoteInput{Content: "old fact about widgets", Cues: []string{"old widgets"}})
	if err != nil {
		t.Fatalf("note: %v", err)
	}
	if err := store.Forget(ctx, ForgetInput{ID: res.ID}); err != nil {
		t.Fatalf("forget: %v", err)
	}
	mock.mu.Lock()
	defer mock.mu.Unlock()
	var meta map[string]any
	_ = json.Unmarshal(mock.messages[0].Metadata, &meta)
	if meta["status"] != statusRetired {
		t.Fatalf("status want retired, got %v", meta["status"])
	}
	deleted := false
	for _, r := range mock.requests {
		if r.method == http.MethodDelete && strings.Contains(r.path, "/conclusions/") {
			deleted = true
		}
	}
	if !deleted {
		t.Fatalf("expected conclusion DELETE")
	}
}

func TestFindRequiresQuery(t *testing.T) {
	_, cli := newHonchoMock(t)
	store := newTestStore(t, cli)
	if _, err := store.Find(context.Background(), FindInput{}); err == nil {
		t.Fatalf("expected error")
	}
}

func TestFindDropsHybridNoiseWhenConclusionsMiss(t *testing.T) {
	mock, cli := newHonchoMock(t)
	store := newTestStore(t, cli)
	ctx := context.Background()
	_, err := store.Note(ctx, NoteInput{
		Content: "orb login shell 不 source /etc/bashrc",
		Cues:    []string{"login shell 没有 nix"},
	})
	if err != nil {
		t.Fatal(err)
	}
	// Semantic miss + hybrid still returns the only message.
	mock.mu.Lock()
	mock.nextQuery = []*honcho.Conclusion{}
	mock.nextSearch = append([]honcho.Message(nil), mock.messages...)
	mock.mu.Unlock()

	fr, err := store.search(ctx, "user's favorite pizza topping", 8)
	if err != nil {
		t.Fatal(err)
	}
	if fr.Status != "empty" {
		t.Fatalf("want empty, got %+v", fr)
	}
}

func TestFindKeepsSemanticHitWithoutSharedWords(t *testing.T) {
	mock, cli := newHonchoMock(t)
	store := newTestStore(t, cli)
	ctx := context.Background()
	noted, err := store.Note(ctx, NoteInput{
		Content: "暂停后恢复不会重装依赖。",
		Cues:    []string{"暂停后恢复"},
	})
	if err != nil {
		t.Fatal(err)
	}
	mock.mu.Lock()
	mock.nextQuery = append([]*honcho.Conclusion(nil), mock.conclusions...)
	mock.nextSearch = []honcho.Message{}
	mock.mu.Unlock()

	fr, err := store.search(ctx, "does wake reinstall toolchains", 8)
	if err != nil {
		t.Fatal(err)
	}
	if fr.Status != "ok" || len(fr.Hits) == 0 {
		t.Fatalf("semantic hit should survive, got %+v", fr)
	}
	if fr.Hits[0].ID != noted.ID {
		t.Fatalf("hit id want %s, got %+v", noted.ID, fr.Hits[0])
	}
	if fr.Hits[0].Source != "conclusion" && fr.Hits[0].Source != "both" {
		t.Fatalf("want conclusion-backed source, got %s", fr.Hits[0].Source)
	}
}

func TestAskGroundedFromSearch(t *testing.T) {
	mock, cli := newHonchoMock(t)
	mock.chatContent = `{"answer":"","unknown":true,"quotes":[]}`
	store := newTestStore(t, cli)
	ctx := context.Background()
	_, err := store.Note(ctx, NoteInput{
		Content: "orb login shell 不 source /etc/bashrc",
		Cues:    []string{"login shell 没有 nix"},
	})
	if err != nil {
		t.Fatal(err)
	}
	ar, err := store.Find(ctx, FindInput{Query: "login shell 没有 nix"})
	if err != nil {
		t.Fatal(err)
	}
	if ar.Unknown {
		t.Fatalf("search fallback should answer")
	}
	if !strings.Contains(ar.Answer, "bashrc") {
		t.Fatalf("answer: %q", ar.Answer)
	}
}

func TestFindRecallsByCueWhenHonchoMisses(t *testing.T) {
	mock, cli := newHonchoMock(t)
	store := newTestStore(t, cli)
	ctx := context.Background()
	noted, err := store.Note(ctx, NoteInput{
		Content: "暂停后恢复不会重装依赖。",
		Cues:    []string{"wake reinstall toolchains"},
	})
	if err != nil {
		t.Fatal(err)
	}
	mock.mu.Lock()
	mock.nextQuery = []*honcho.Conclusion{}
	mock.nextSearch = []honcho.Message{}
	mock.mu.Unlock()

	fr, err := store.search(ctx, "wake reinstall toolchains", 8)
	if err != nil {
		t.Fatal(err)
	}
	if fr.Status != "ok" || len(fr.Hits) == 0 {
		t.Fatalf("cue recall should hit, got %+v", fr)
	}
	if fr.Hits[0].ID != noted.ID {
		t.Fatalf("want %s, got %+v", noted.ID, fr.Hits[0])
	}
}

func TestAskUnknownSkipsChat(t *testing.T) {
	mock, cli := newHonchoMock(t)
	mock.chatContent = `{"answer":"I invented a pizza fact","unknown":false}`
	store := newTestStore(t, cli)
	ctx := context.Background()
	if _, err := store.Note(ctx, NoteInput{Content: "orb login shell 不 source /etc/bashrc", Cues: []string{"login shell 没有 nix"}}); err != nil {
		t.Fatal(err)
	}
	mock.mu.Lock()
	mock.nextQuery = []*honcho.Conclusion{}
	mock.nextSearch = []honcho.Message{}
	mock.mu.Unlock()

	ar, err := store.Find(ctx, FindInput{Query: "user's favorite pizza topping"})
	if err != nil {
		t.Fatal(err)
	}
	if !ar.Unknown || ar.Answer != "" {
		t.Fatalf("want unknown empty, got %+v", ar)
	}
	mock.mu.Lock()
	defer mock.mu.Unlock()
	for _, r := range mock.requests {
		if strings.HasSuffix(r.path, "/chat") {
			t.Fatalf("empty search must not call peer.chat")
		}
	}
}

func TestListLiveEntriesPagesPast100(t *testing.T) {
	mock, cli := newHonchoMock(t)
	store := newTestStore(t, cli)
	ctx := context.Background()
	// 105 notes; cue on the oldest so page 2 must be scanned.
	var oldest *NoteResult
	for i := 0; i < 105; i++ {
		in := NoteInput{Content: "note body " + strconv.Itoa(i), Cues: []string{"note-body-" + strconv.Itoa(i)}}
		if i == 0 {
			in.Cues = []string{"unique-oldest-cue"}
		}
		res, err := store.Note(ctx, in)
		if err != nil {
			t.Fatal(err)
		}
		if i == 0 {
			oldest = res
		}
	}
	mock.mu.Lock()
	mock.nextQuery = []*honcho.Conclusion{}
	mock.nextSearch = []honcho.Message{}
	mock.mu.Unlock()

	fr, err := store.search(ctx, "unique-oldest-cue", 8)
	if err != nil {
		t.Fatal(err)
	}
	if fr.Status != "ok" || len(fr.Hits) == 0 {
		t.Fatalf("oldest cue should be recalled across pages, got %+v", fr)
	}
	if fr.Hits[0].ID != oldest.ID {
		t.Fatalf("want oldest %s, got %+v", oldest.ID, fr.Hits[0])
	}
}

func TestForgetDeletesConclusionByContent(t *testing.T) {
	mock, cli := newHonchoMock(t)
	store := newTestStore(t, cli)
	ctx := context.Background()
	res, err := store.Note(ctx, NoteInput{Content: "forget me please", Cues: []string{"forget me"}})
	if err != nil {
		t.Fatal(err)
	}
	// Simulate a failed metadata patch: strip conclusion_id from the message.
	mock.mu.Lock()
	var meta map[string]any
	_ = json.Unmarshal(mock.messages[0].Metadata, &meta)
	delete(meta, "conclusion_id")
	raw, _ := json.Marshal(meta)
	mock.messages[0].Metadata = raw
	mock.mu.Unlock()

	if err := store.Forget(ctx, ForgetInput{ID: res.ID}); err != nil {
		t.Fatalf("forget: %v", err)
	}
	mock.mu.Lock()
	defer mock.mu.Unlock()
	if len(mock.conclusions) != 0 {
		t.Fatalf("leftover conclusions: %+v", mock.conclusions)
	}
}
