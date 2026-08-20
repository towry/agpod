package memo

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/url"
	"sort"
	"strings"
	"sync"
	"time"

	honcho "github.com/hekmon/go-honcho"

	"github.com/towry/agpod/internal/agpod-mcp/internal/config"
)

// SessionPrefix is prepended to repo_id to form the Honcho session ID.
// Honcho enforces `^[a-zA-Z0-9_-]+$` on session IDs; repo_id is hex so the
// combined value satisfies the constraint.
const SessionPrefix = "memo_"

// Store persists and retrieves memory entries via Honcho v3.
type Store struct {
	cli         *honcho.Client
	workspaceID string
	peerID      string
	repoID      string
	repoLabel   string

	now func() time.Time
	id  func() string
}

// Options configures Store creation.
type Options struct {
	Workspace string
	PeerID    string
	RepoID    string
	RepoLabel string

	Now func() time.Time
	ID  func() string
}

// NewStore constructs a Store backed by the given Honcho client.
func NewStore(cli *honcho.Client, opts Options) (*Store, error) {
	if cli == nil {
		return nil, errors.New("honcho client is nil")
	}
	if opts.Workspace == "" {
		return nil, errors.New("workspace id is required")
	}
	if opts.PeerID == "" {
		return nil, errors.New("peer id is required")
	}
	if opts.RepoID == "" {
		return nil, errors.New("repo id is required")
	}
	s := &Store{
		cli:         cli,
		workspaceID: opts.Workspace,
		peerID:      opts.PeerID,
		repoID:      opts.RepoID,
		repoLabel:   opts.RepoLabel,
		now:         opts.Now,
		id:          opts.ID,
	}
	if s.now == nil {
		s.now = func() time.Time { return time.Now().UTC() }
	}
	if s.id == nil {
		s.id = newUUID
	}
	return s, nil
}

// NewClient constructs a honcho.Client from config.Config.
func NewClient(cfg config.Config) (*honcho.Client, error) {
	base, err := url.Parse(cfg.HonchoBaseURL)
	if err != nil {
		return nil, fmt.Errorf("parse honcho base url: %w", err)
	}
	return honcho.New(&honcho.Options{
		APIKey:  cfg.HonchoAPIKey,
		BaseURL: base,
	}), nil
}

// SessionID returns the Honcho session id used for this repo.
func (s *Store) SessionID() string { return SessionPrefix + s.repoID }

// RepoID returns the bound repo id.
func (s *Store) RepoID() string { return s.repoID }

// Ensure creates the peer and session if missing. Safe to call repeatedly.
func (s *Store) Ensure(ctx context.Context) error {
	observeMe := true
	observeOthers := false
	if _, err := s.cli.GetOrCreatePeer(ctx, s.workspaceID, honcho.PeerCreate{
		ID: s.peerID,
		Configuration: honcho.PeerConfig{
			ObserveMe:     &observeMe,
			ObserveOthers: &observeOthers,
		},
	}); err != nil {
		return fmt.Errorf("ensure peer: %w", err)
	}
	meta, err := json.Marshal(map[string]any{
		"repo_id":    s.repoID,
		"repo_label": s.repoLabel,
	})
	if err != nil {
		return fmt.Errorf("marshal session metadata: %w", err)
	}
	sid := s.SessionID()
	_, err = s.cli.GetOrCreateSession(ctx, s.workspaceID, honcho.SessionCreate{
		ID:       sid,
		Metadata: meta,
		Peers: map[string]*honcho.SessionPeerConfig{
			s.peerID: {ObserveMe: &observeMe, ObserveOthers: &observeOthers},
		},
	})
	if err != nil {
		return fmt.Errorf("ensure session: %w", err)
	}
	return nil
}

// Note stores a standalone fact. Returns the new entry id.
func (s *Store) Note(ctx context.Context, in NoteInput) (*NoteResult, error) {
	content := collapseSpace(in.Content)
	if content == "" {
		return nil, errors.New("content is required")
	}
	manual := cleanCues(in.Cues)
	if len(manual) == 0 {
		return nil, errors.New("cues must include at least one short search phrase")
	}
	cues := mergeCues(manual, ExtractTokens(content))
	entryID := s.id()
	created := s.now()
	e := entry{
		EntryID:   entryID,
		RepoID:    s.repoID,
		Status:    statusLive,
		Content:   content,
		Cues:      cues,
		CreatedAt: created,
	}
	meta, err := entryMetadata(e)
	if err != nil {
		return nil, err
	}
	msgs, err := s.cli.CreateMessagesForSession(ctx, s.workspaceID, s.SessionID(), honcho.MessageBatchCreate{
		Messages: []honcho.MessageCreate{{
			Content:   messageBody(content, cues),
			PeerID:    s.peerID,
			Metadata:  meta,
			CreatedAt: &created,
		}},
	})
	if err != nil {
		return nil, fmt.Errorf("create message: %w", err)
	}
	if len(msgs) > 0 {
		e.MessageID = msgs[0].ID
		e.SessionID = msgs[0].SessionID
	}

	sid := s.SessionID()
	conclusions, err := s.cli.CreateConclusions(ctx, s.workspaceID, honcho.ConclusionBatchCreate{
		Conclusions: []honcho.ConclusionCreate{{
			Content:    content,
			ObserverID: s.peerID,
			ObservedID: s.peerID,
			SessionID:  &sid,
		}},
	})
	if err != nil {
		return &NoteResult{ID: entryID, Indexed: false}, nil
	}
	if len(conclusions) == 0 || conclusions[0] == nil || conclusions[0].ID == "" {
		return &NoteResult{ID: entryID, Indexed: false}, nil
	}
	e.ConclusionID = conclusions[0].ID
	patched, err := entryMetadata(e)
	if err != nil {
		return &NoteResult{ID: entryID, Indexed: true}, nil
	}
	if e.MessageID != "" {
		if _, patchErr := s.cli.UpdateMessage(ctx, s.workspaceID, s.SessionID(), e.MessageID, honcho.MessageUpdate{
			Metadata: patched,
		}); patchErr != nil {
			return &NoteResult{ID: entryID, Indexed: true}, nil
		}
	}
	return &NoteResult{ID: entryID, Indexed: true}, nil
}

// Find answers from stored notes. It retrieves ranked hits internally, then
// asks Honcho to synthesize. Empty retrieval returns unknown without chat.
func (s *Store) Find(ctx context.Context, in FindInput) (*AskResult, error) {
	query := collapseSpace(in.Query)
	if query == "" {
		return nil, errors.New("query is required")
	}
	return s.ask(ctx, query)
}

func (s *Store) search(ctx context.Context, query string, limit int) (*FindResult, error) {
	if limit <= 0 {
		limit = 8
	}
	if limit > 50 {
		limit = 50
	}

	sid := s.SessionID()
	searchLimit := limit
	if searchLimit < 1 {
		searchLimit = 1
	}
	maxDist := 0.55

	var (
		conclusions []*honcho.Conclusion
		concErr     error
		msgs        []honcho.Message
		msgErr      error
		live        []entry
		liveErr     error
		wg          sync.WaitGroup
	)
	wg.Add(3)
	go func() {
		defer wg.Done()
		conclusions, concErr = s.queryConclusionsOnce(ctx, query, limit, maxDist, sid)
		if concErr != nil {
			conclusions, concErr = s.queryConclusionsOnce(ctx, query, limit, maxDist, sid)
		}
	}()
	go func() {
		defer wg.Done()
		cctx, cancel := context.WithTimeout(ctx, 2*time.Second)
		defer cancel()
		msgs, msgErr = s.cli.SearchSession(cctx, s.workspaceID, sid, honcho.MessageSearchOptions{
			Query: query,
			Limit: searchLimit,
			Filters: map[string]any{
				"metadata": map[string]any{"status": statusLive},
			},
		})
	}()
	go func() {
		defer wg.Done()
		cctx, cancel := context.WithTimeout(ctx, 3*time.Second)
		defer cancel()
		live, liveErr = s.listLiveEntries(cctx)
	}()
	wg.Wait()

	if liveErr != nil {
		return nil, liveErr
	}
	if msgErr != nil && concErr != nil {
		return nil, fmt.Errorf("search session: %w; query conclusions: %v", msgErr, concErr)
	}
	if concErr != nil {
		conclusions = nil
	}
	byConclusion := map[string]entry{}
	byContent := map[string]entry{}
	byID := map[string]entry{}
	for _, e := range live {
		byID[e.EntryID] = e
		if e.ConclusionID != "" {
			byConclusion[e.ConclusionID] = e
		}
		byContent[normalizeKey(e.Content)] = e
	}

	type ranked struct {
		e       entry
		source  string
		overlap int
		order   int
	}
	seen := map[string]int{} // entry_id -> index in out
	var out []ranked
	add := func(e entry, source string) {
		if e.EntryID == "" || e.Status != statusLive {
			return
		}
		if i, ok := seen[e.EntryID]; ok {
			out[i].source = mergeSource(out[i].source, source)
			if source == "cue" && out[i].overlap < 2 {
				out[i].overlap = 2
			}
			return
		}
		seen[e.EntryID] = len(out)
		out = append(out, ranked{
			e:       e,
			source:  source,
			overlap: cueOverlap(query, e.Cues),
			order:   len(out),
		})
	}

	for _, c := range conclusions {
		if c == nil {
			continue
		}
		if e, ok := byConclusion[c.ID]; ok {
			add(e, "conclusion")
			continue
		}
		if e, ok := byContent[normalizeKey(c.Content)]; ok {
			add(e, "conclusion")
			continue
		}
		// Conclusion hit without a live message: still surface the clean body.
		synthetic := entry{
			Content:   c.Content,
			CreatedAt: c.CreatedAt,
			Status:    statusLive,
			EntryID:   "", // unknown
		}
		// Use content as a temporary key so duplicates collapse.
		key := "content:" + normalizeKey(c.Content)
		if _, ok := seen[key]; ok {
			continue
		}
		seen[key] = len(out)
		out = append(out, ranked{
			e:       synthetic,
			source:  "conclusion",
			overlap: cueOverlap(query, ExtractTokens(c.Content)),
			order:   len(out),
		})
	}
	for i := range msgs {
		e, decErr := decodeEntry(&msgs[i])
		if decErr != nil || e.EntryID == "" {
			continue
		}
		if e.Status != statusLive {
			continue
		}
		if hydrated, ok := byID[e.EntryID]; ok {
			add(hydrated, "message")
			continue
		}
		add(e, "message")
	}

	// Local recall: inject live notes Honcho missed when the query covers
	// a cue, or when CJK content shares a 3+ character run (same-language
	// paraphrase without a cue).
	for _, e := range live {
		if cueOverlap(query, e.Cues) >= 2 {
			add(e, "cue")
			continue
		}
		if contentOverlap(query, e.Content) >= 1 {
			add(e, "content")
		}
	}

	filtered := out[:0]
	for _, r := range out {
		// Keep Honcho conclusion hits even with no shared words — that is
		// the semantic path. Drop message-only hybrid noise that shares
		// neither a cue nor a content token with the query.
		if r.source == "message" && r.overlap == 0 && contentOverlap(query, r.e.Content) == 0 {
			continue
		}
		filtered = append(filtered, r)
	}
	out = filtered

	sort.SliceStable(out, func(i, j int) bool {
		if out[i].overlap != out[j].overlap {
			return out[i].overlap > out[j].overlap
		}
		// Prefer conclusion-backed hits over hybrid-only when overlap ties.
		si, sj := sourceRank(out[i].source), sourceRank(out[j].source)
		if si != sj {
			return si > sj
		}
		return out[i].order < out[j].order
	})
	if len(out) > limit {
		out = out[:limit]
	}

	hits := make([]FindHit, 0, len(out))
	for i, r := range out {
		hits = append(hits, FindHit{
			ID:        r.e.EntryID,
			Content:   r.e.Content,
			Cues:      r.e.Cues,
			Source:    r.source,
			Rank:      i + 1,
			CreatedAt: r.e.CreatedAt,
		})
	}
	status := "empty"
	if len(hits) > 0 {
		status = "ok"
	}
	return &FindResult{Status: status, Hits: hits}, nil
}

func (s *Store) ask(ctx context.Context, query string) (*AskResult, error) {
	fr, err := s.search(ctx, query, 3)
	if err != nil {
		return nil, err
	}
	if fr == nil || len(fr.Hits) == 0 {
		return &AskResult{Unknown: true}, nil
	}

	sid := s.SessionID()
	prompt := "Answer this question using only the stored repository notes. " +
		"Quote the relevant facts. Do not invent.\n\nQuestion: " + query
	cctx, cancel := context.WithTimeout(ctx, 8*time.Second)
	defer cancel()
	resp, chatErr := s.cli.Chat(cctx, s.workspaceID, s.peerID, honcho.DialecticOptions{
		Query:          prompt,
		SessionID:      &sid,
		ReasoningLevel: honcho.ReasoningLevelLow,
	})
	if chatErr == nil && resp != nil && resp.Content != nil {
		raw := strings.TrimSpace(*resp.Content)
		if parsed := parseAskJSON(raw); parsed != nil {
			if !parsed.Unknown && parsed.Answer != "" {
				for _, h := range fr.Hits {
					if h.Content != "" {
						parsed.Quotes = append(parsed.Quotes, h.Content)
					}
					if h.ID != "" {
						parsed.IDs = append(parsed.IDs, h.ID)
					}
				}
				return parsed, nil
			}
		} else if raw != "" && !looksUnknown(raw) {
			out := &AskResult{Answer: raw, Unknown: false}
			for _, h := range fr.Hits {
				if h.Content != "" {
					out.Quotes = append(out.Quotes, h.Content)
				}
				if h.ID != "" {
					out.IDs = append(out.IDs, h.ID)
				}
			}
			return out, nil
		}
	}
	return groundedFromSearch(fr), nil
}

func groundedFromSearch(fr *FindResult) *AskResult {
	quotes := make([]string, 0, len(fr.Hits))
	ids := make([]string, 0, len(fr.Hits))
	for _, h := range fr.Hits {
		if h.Content != "" {
			quotes = append(quotes, h.Content)
		}
		if h.ID != "" {
			ids = append(ids, h.ID)
		}
	}
	return &AskResult{
		Answer:  fr.Hits[0].Content,
		Unknown: false,
		Quotes:  quotes,
		IDs:     ids,
	}
}

func parseAskJSON(raw string) *AskResult {
	raw = strings.TrimSpace(raw)
	if i := strings.Index(raw, "{"); i >= 0 {
		if j := strings.LastIndex(raw, "}"); j > i {
			raw = raw[i : j+1]
		}
	}
	var probe struct {
		Answer  *string `json:"answer"`
		Unknown *bool   `json:"unknown"`
	}
	if err := json.Unmarshal([]byte(raw), &probe); err != nil {
		return nil
	}
	if probe.Answer == nil && probe.Unknown == nil {
		return nil
	}
	out := &AskResult{}
	if probe.Answer != nil {
		out.Answer = strings.TrimSpace(*probe.Answer)
	}
	if probe.Unknown != nil {
		out.Unknown = *probe.Unknown
	}
	return out
}

func looksUnknown(s string) bool {
	n := strings.ToLower(s)
	needles := []string{
		"no stored", "nothing relevant", "i don't know", "i do not know",
		"i don’t have any information", "no memory", "not recorded",
		"没有记录", "不知道",
	}
	for _, n0 := range needles {
		if strings.Contains(n, n0) {
			return true
		}
	}
	return false
}

func (s *Store) queryConclusionsOnce(ctx context.Context, query string, limit int, maxDist float64, sid string) ([]*honcho.Conclusion, error) {
	cctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	return s.cli.QueryConclusions(cctx, s.workspaceID, honcho.ConclusionQuery{
		Query:    query,
		TopK:     limit,
		Distance: &maxDist,
		Filters: map[string]any{
			"session_id":  sid,
			"observer_id": s.peerID,
			"observed_id": s.peerID,
			"level":       "explicit",
		},
	})
}

func sourceRank(source string) int {
	switch source {
	case "both", "cue":
		return 2
	case "conclusion", "content":
		return 1
	default:
		return 0
	}
}

func mergeSource(existing, incoming string) string {
	if existing == incoming {
		return existing
	}
	if existing == "cue" || incoming == "cue" {
		return "cue"
	}
	if existing == "both" || incoming == "both" {
		return "both"
	}
	return "both"
}

func (s *Store) Forget(ctx context.Context, in ForgetInput) error {
	id := strings.TrimSpace(in.ID)
	if id == "" {
		return errors.New("id is required")
	}
	msg, err := s.findMessageByEntryID(ctx, id)
	if err != nil {
		return err
	}
	if msg == nil {
		return fmt.Errorf("entry %s not found", id)
	}
	e, err := decodeEntry(msg)
	if err != nil {
		return err
	}
	if e.ConclusionID != "" {
		if delErr := s.cli.DeleteConclusion(ctx, s.workspaceID, e.ConclusionID); delErr != nil {
			return fmt.Errorf("delete conclusion: %w", delErr)
		}
	}
	e.Status = statusRetired
	meta, err := entryMetadata(e)
	if err != nil {
		return err
	}
	if _, err := s.cli.UpdateMessage(ctx, s.workspaceID, msg.SessionID, msg.ID, honcho.MessageUpdate{
		Metadata: meta,
	}); err != nil {
		return fmt.Errorf("mark retired: %w", err)
	}
	return nil
}

func (s *Store) listLiveEntries(ctx context.Context) ([]entry, error) {
	var out []entry
	err := s.forEachMessage(ctx, func(msg *honcho.Message) bool {
		e, decErr := decodeEntry(msg)
		if decErr != nil || e.EntryID == "" || e.Status != statusLive {
			return true
		}
		out = append(out, e)
		return true
	})
	if err != nil {
		return nil, err
	}
	return out, nil
}

func (s *Store) findMessageByEntryID(ctx context.Context, entryID string) (*honcho.Message, error) {
	var found *honcho.Message
	err := s.forEachMessage(ctx, func(msg *honcho.Message) bool {
		e, decErr := decodeEntry(msg)
		if decErr == nil && e.EntryID == entryID {
			cp := *msg
			found = &cp
			return false
		}
		return true
	})
	if err != nil {
		return nil, fmt.Errorf("list for entry_id %s: %w", entryID, err)
	}
	return found, nil
}

const messagePageSize = 100
const messagePageCap = 50

func (s *Store) forEachMessage(ctx context.Context, fn func(*honcho.Message) bool) error {
	for pageNum := 1; pageNum <= messagePageCap; pageNum++ {
		page, err := s.cli.GetMessages(ctx, s.workspaceID, s.SessionID(), nil, &honcho.GetMessagesOptions{
			Size:    messagePageSize,
			Reverse: true,
			Page:    pageNum,
		})
		if err != nil {
			return fmt.Errorf("list session messages: %w", err)
		}
		if page == nil || len(page.Items) == 0 {
			return nil
		}
		for i := range page.Items {
			if !fn(&page.Items[i]) {
				return nil
			}
		}
		if page.Pages > 0 && pageNum >= page.Pages {
			return nil
		}
		if page.Pages == 0 && len(page.Items) < messagePageSize {
			return nil
		}
	}
	return nil
}

func entryMetadata(e entry) (json.RawMessage, error) {
	m := map[string]any{
		"schema":     schemaV1,
		"entry_id":   e.EntryID,
		"repo_id":    e.RepoID,
		"status":     e.Status,
		"created_at": e.CreatedAt.Format(time.RFC3339Nano),
	}
	if len(e.Cues) > 0 {
		m["cues"] = e.Cues
	}
	if e.ConclusionID != "" {
		m["conclusion_id"] = e.ConclusionID
	}
	return json.Marshal(m)
}

func decodeEntry(msg *honcho.Message) (entry, error) {
	meta := decodeRawMetadata(msg.Metadata)
	e := entry{
		EntryID:      stringField(meta, "entry_id"),
		RepoID:       stringField(meta, "repo_id"),
		Status:       stringField(meta, "status"),
		Content:      stripAppendix(msg.Content),
		Cues:         stringSliceField(meta, "cues"),
		ConclusionID: stringField(meta, "conclusion_id"),
		MessageID:    msg.ID,
		SessionID:    msg.SessionID,
	}
	if e.Status == "" {
		e.Status = statusLive
	}
	if ts := stringField(meta, "created_at"); ts != "" {
		if parsed, err := time.Parse(time.RFC3339Nano, ts); err == nil {
			e.CreatedAt = parsed
		}
	}
	if e.CreatedAt.IsZero() {
		e.CreatedAt = msg.CreatedAt
	}
	return e, nil
}

func decodeRawMetadata(raw json.RawMessage) map[string]any {
	if len(raw) == 0 {
		return map[string]any{}
	}
	var m map[string]any
	if err := json.Unmarshal(raw, &m); err != nil {
		return map[string]any{}
	}
	return m
}

func stringField(m map[string]any, key string) string {
	if v, ok := m[key].(string); ok {
		return v
	}
	return ""
}

func stringSliceField(m map[string]any, key string) []string {
	raw, ok := m[key]
	if !ok {
		return nil
	}
	arr, ok := raw.([]any)
	if !ok {
		return nil
	}
	out := make([]string, 0, len(arr))
	for _, v := range arr {
		if s, ok := v.(string); ok {
			out = append(out, s)
		}
	}
	return out
}
