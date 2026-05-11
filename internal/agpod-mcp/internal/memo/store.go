package memo

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/url"
	"sort"
	"strings"
	"time"

	honcho "github.com/hekmon/go-honcho"

	"github.com/towry/agpod/internal/agpod-mcp/internal/config"
)

// SessionPrefix is prepended to repo_id to form the Honcho session ID.
// Honcho enforces `^[a-zA-Z0-9_-]+$` on session IDs; repo_id is hex so the
// combined value satisfies the constraint.
const SessionPrefix = "memo_"

// Store persists and retrieves memo entries via Honcho v3.
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

	// Now/ID let tests inject deterministic clocks and UUIDs.
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
	if _, err := s.cli.GetOrCreatePeer(ctx, s.workspaceID, honcho.PeerCreate{ID: s.peerID}); err != nil {
		return fmt.Errorf("ensure peer: %w", err)
	}
	meta, err := json.Marshal(map[string]any{
		"repo_id":    s.repoID,
		"repo_label": s.repoLabel,
	})
	if err != nil {
		return fmt.Errorf("marshal session metadata: %w", err)
	}
	_, err = s.cli.GetOrCreateSession(ctx, s.workspaceID, honcho.SessionCreate{
		ID:       s.SessionID(),
		Metadata: meta,
	})
	if err != nil {
		return fmt.Errorf("ensure session: %w", err)
	}
	return nil
}

// WriteFinding stores a finding entry. Returns the new entry id.
func (s *Store) WriteFinding(ctx context.Context, in WriteFindingInput) (string, error) {
	if strings.TrimSpace(in.Content) == "" {
		return "", errors.New("content is required")
	}
	if len(in.Scope) == 0 {
		return "", errors.New("scope must include at least one anchor")
	}
	entry := s.newEntry(EntryFinding, in.Content)
	entry.Scope = cleanStrings(in.Scope)
	entry.EvidenceRefs = cleanStrings(in.EvidenceRefs)
	if err := s.createMessage(ctx, entry); err != nil {
		return "", err
	}
	return entry.EntryID, nil
}

// WriteDecision stores a decision entry. If Supersedes is set the old entry is
// marked superseded (best-effort) before the new entry is persisted.
func (s *Store) WriteDecision(ctx context.Context, in WriteDecisionInput) (string, error) {
	if strings.TrimSpace(in.Content) == "" {
		return "", errors.New("content is required")
	}
	if len(in.Scope) == 0 {
		return "", errors.New("scope must include at least one anchor")
	}
	entry := s.newEntry(EntryDecision, in.Content)
	entry.Scope = cleanStrings(in.Scope)
	entry.EvidenceRefs = cleanStrings(in.EvidenceRefs)
	entry.RejectedAlternatives = cleanAlternatives(in.RejectedAlternatives)
	entry.TriggerEvidences = cleanStrings(in.TriggerEvidences)
	entry.Constraints = cleanStrings(in.Constraints)
	entry.Supersedes = strings.TrimSpace(in.Supersedes)
	entry.SupersedeReason = strings.TrimSpace(in.SupersedeReason)
	if entry.Supersedes != "" {
		if err := s.SetStatus(ctx, SetStatusInput{
			EntryID: entry.Supersedes,
			Status:  StatusSuperseded,
			Reason:  entry.SupersedeReason,
		}); err != nil {
			return "", fmt.Errorf("mark old decision superseded: %w", err)
		}
	}
	if err := s.createMessage(ctx, entry); err != nil {
		return "", err
	}
	return entry.EntryID, nil
}

// WriteHandoff stores a handoff entry.
func (s *Store) WriteHandoff(ctx context.Context, in WriteHandoffInput) (string, error) {
	if strings.TrimSpace(in.Summary) == "" {
		return "", errors.New("summary is required")
	}
	if strings.TrimSpace(in.Content) == "" {
		return "", errors.New("content is required")
	}
	entry := s.newEntry(EntryHandoff, in.Content)
	entry.Summary = in.Summary
	if err := s.createMessage(ctx, entry); err != nil {
		return "", err
	}
	return entry.EntryID, nil
}

// PickupHandoff returns either a handoff by id or the most recent live handoff.
func (s *Store) PickupHandoff(ctx context.Context, in PickupHandoffInput) (*PickupHandoffResult, error) {
	if id := strings.TrimSpace(in.HandoffID); id != "" {
		msg, err := s.findMessageByEntryID(ctx, id, in.CrossRepo)
		if err != nil {
			return nil, err
		}
		if msg == nil {
			return nil, fmt.Errorf("handoff %s not found", id)
		}
		entry, err := decodeEntry(msg)
		if err != nil {
			return nil, err
		}
		if entry.EntryType != EntryHandoff {
			return nil, fmt.Errorf("entry %s is not a handoff", id)
		}
		return handoffResult(entry), nil
	}

	if in.CrossRepo {
		return nil, fmt.Errorf("memo_pickup_handoff: %w; pass handoff_id to fetch a specific cross-repo handoff", ErrCrossRepoRequiresQuery)
	}
	msgs, err := s.listMessages(ctx, false)
	if err != nil {
		return nil, err
	}
	entries := decodeEntries(msgs)
	var best *Entry
	for i := range entries {
		e := &entries[i]
		if e.EntryType != EntryHandoff || e.Status != StatusLive {
			continue
		}
		if best == nil || e.CreatedAt.After(best.CreatedAt) {
			best = e
		}
	}
	if best == nil {
		return nil, errors.New("no live handoff entries found")
	}
	return handoffResult(*best), nil
}

// Recall returns hits matching the query. When query is empty the most recent
// entries are returned. Honcho's metadata filters are best-effort: results are
// post-filtered locally for status/scope_prefix to remain robust.
func (s *Store) Recall(ctx context.Context, in RecallInput) ([]RecallHit, error) {
	limit := in.Limit
	if limit <= 0 {
		limit = 20
	}
	if limit > 100 {
		limit = 100
	}

	var (
		hits []RecallHit
		err  error
	)
	query := strings.TrimSpace(in.Query)
	if query != "" {
		// Over-fetch to leave headroom for post-filtering; cap at Honcho's max.
		overFetch := limit * 3
		if overFetch > 100 {
			overFetch = 100
		}
		hits, err = s.semanticSearch(ctx, query, overFetch, in.CrossRepo)
		if err != nil {
			return nil, err
		}
	} else {
		msgs, listErr := s.listMessages(ctx, in.CrossRepo)
		if listErr != nil {
			return nil, listErr
		}
		entries := decodeEntries(msgs)
		hits = entriesToHits(entries, msgs, 0)
	}

	filtered := filterHits(hits, in)
	if len(filtered) > limit {
		filtered = filtered[:limit]
	}
	return filtered, nil
}

// Why returns the decision graph for a scope anchor.
func (s *Store) Why(ctx context.Context, in WhyInput) (*WhyResult, error) {
	scope := strings.TrimSpace(in.Scope)
	if scope == "" {
		return nil, errors.New("scope is required")
	}
	msgs, err := s.listMessages(ctx, in.CrossRepo)
	if err != nil {
		return nil, err
	}
	entries := decodeEntries(msgs)
	return buildWhy(entries, scope), nil
}

// SetStatus updates the status metadata of an existing entry.
func (s *Store) SetStatus(ctx context.Context, in SetStatusInput) error {
	id := strings.TrimSpace(in.EntryID)
	if id == "" {
		return errors.New("entry_id is required")
	}
	if !in.Status.Valid() || in.Status == StatusLive {
		return fmt.Errorf("status must be superseded or no_longer_applicable, got %q", in.Status)
	}
	msg, err := s.findMessageByEntryID(ctx, id, false)
	if err != nil {
		return err
	}
	if msg == nil {
		// Fall back to a cross-repo search so superseding works for borrowed entries.
		msg, err = s.findMessageByEntryID(ctx, id, true)
		if err != nil {
			return err
		}
	}
	if msg == nil {
		return fmt.Errorf("entry %s not found", id)
	}
	meta := decodeRawMetadata(msg.Metadata)
	meta["status"] = string(in.Status)
	if reason := strings.TrimSpace(in.Reason); reason != "" {
		meta["status_reason"] = reason
	}
	meta["status_changed_at"] = s.now().Format(time.RFC3339Nano)
	encoded, err := json.Marshal(meta)
	if err != nil {
		return fmt.Errorf("marshal updated metadata: %w", err)
	}
	if _, err := s.cli.UpdateMessage(ctx, s.workspaceID, msg.SessionID, msg.ID, honcho.MessageUpdate{
		Metadata: encoded,
	}); err != nil {
		return fmt.Errorf("update message metadata: %w", err)
	}
	return nil
}

// --- internals ---

func (s *Store) newEntry(t EntryType, content string) Entry {
	return Entry{
		EntryID:   s.id(),
		EntryType: t,
		RepoID:    s.repoID,
		Status:    StatusLive,
		CreatedAt: s.now(),
		Content:   content,
	}
}

func (s *Store) createMessage(ctx context.Context, entry Entry) error {
	meta, err := entryMetadata(entry)
	if err != nil {
		return err
	}
	created := entry.CreatedAt
	_, err = s.cli.CreateMessagesForSession(ctx, s.workspaceID, s.SessionID(), honcho.MessageBatchCreate{
		Messages: []honcho.MessageCreate{
			{
				Content:   entry.Content,
				PeerID:    s.peerID,
				Metadata:  meta,
				CreatedAt: &created,
			},
		},
	})
	if err != nil {
		return fmt.Errorf("create message: %w", err)
	}
	return nil
}

// ErrCrossRepoRequiresQuery is returned when an operation that needs to look
// outside the current repo session is invoked without a search query. Honcho
// has no documented "list every workspace message" endpoint, so we refuse
// rather than silently misbehave.
var ErrCrossRepoRequiresQuery = errors.New("cross_repo listing requires a non-empty query")

func (s *Store) listMessages(ctx context.Context, crossRepo bool) ([]honcho.Message, error) {
	if crossRepo {
		return nil, ErrCrossRepoRequiresQuery
	}
	page, err := s.cli.GetMessages(ctx, s.workspaceID, s.SessionID(), nil, &honcho.GetMessagesOptions{
		Size:    100,
		Reverse: true,
	})
	if err != nil {
		return nil, fmt.Errorf("list session messages: %w", err)
	}
	return page.Items, nil
}

func (s *Store) semanticSearch(ctx context.Context, query string, limit int, crossRepo bool) ([]RecallHit, error) {
	if limit < 1 {
		limit = 1
	}
	if limit > 100 {
		limit = 100
	}
	opts := honcho.MessageSearchOptions{Query: query, Limit: limit}
	var msgs []honcho.Message
	if crossRepo {
		res, err := s.cli.SearchWorkspace(ctx, s.workspaceID, opts)
		if err != nil {
			return nil, fmt.Errorf("workspace search: %w", err)
		}
		if res != nil {
			msgs = *res
		}
	} else {
		res, err := s.cli.SearchSession(ctx, s.workspaceID, s.SessionID(), opts)
		if err != nil {
			return nil, fmt.Errorf("session search: %w", err)
		}
		msgs = res
	}
	entries := decodeEntries(msgs)
	return entriesToHits(entries, msgs, 1.0), nil
}

// findMessageByEntryID locates an entry by its UUID. Honcho's metadata-filter
// support is uneven across versions, so we always do a bounded scan of the
// session's newest 100 messages — predictable and avoids silently swallowing a
// failed filter call. Cross-repo lookup uses the workspace search with the
// entry_id as a literal query token.
func (s *Store) findMessageByEntryID(ctx context.Context, entryID string, crossRepo bool) (*honcho.Message, error) {
	if !crossRepo {
		page, err := s.cli.GetMessages(ctx, s.workspaceID, s.SessionID(), nil, &honcho.GetMessagesOptions{
			Size: 100, Reverse: true,
		})
		if err != nil {
			return nil, fmt.Errorf("list for entry_id %s: %w", entryID, err)
		}
		for i := range page.Items {
			e, decErr := decodeEntry(&page.Items[i])
			if decErr == nil && e.EntryID == entryID {
				return &page.Items[i], nil
			}
		}
		return nil, nil
	}
	res, err := s.cli.SearchWorkspace(ctx, s.workspaceID, honcho.MessageSearchOptions{
		Query: entryID,
		Limit: 50,
	})
	if err != nil {
		return nil, fmt.Errorf("workspace search for entry_id %s: %w", entryID, err)
	}
	if res == nil {
		return nil, nil
	}
	for i := range *res {
		msg := (*res)[i]
		e, decErr := decodeEntry(&msg)
		if decErr == nil && e.EntryID == entryID {
			return &msg, nil
		}
	}
	return nil, nil
}

// entryMetadata serializes an Entry into a flat metadata object, dropping empty
// fields to mirror the Rust adapter's null-stripping policy.
func entryMetadata(e Entry) (json.RawMessage, error) {
	m := map[string]any{
		"entry_id":   e.EntryID,
		"entry_type": string(e.EntryType),
		"repo_id":    e.RepoID,
		"status":     string(e.Status),
		"created_at": e.CreatedAt.Format(time.RFC3339Nano),
	}
	if len(e.Scope) > 0 {
		m["scope"] = e.Scope
	}
	if len(e.EvidenceRefs) > 0 {
		m["evidence_refs"] = e.EvidenceRefs
	}
	if len(e.RejectedAlternatives) > 0 {
		m["rejected_alternatives"] = e.RejectedAlternatives
	}
	if len(e.TriggerEvidences) > 0 {
		m["trigger_evidences"] = e.TriggerEvidences
	}
	if len(e.Constraints) > 0 {
		m["constraints"] = e.Constraints
	}
	if e.Supersedes != "" {
		m["supersedes"] = e.Supersedes
	}
	if e.SupersedeReason != "" {
		m["supersede_reason"] = e.SupersedeReason
	}
	if e.Summary != "" {
		m["summary"] = e.Summary
	}
	return json.Marshal(m)
}

// decodeEntry reads an Entry back from a Honcho message. Best-effort: fields
// not present in metadata are left zero.
func decodeEntry(msg *honcho.Message) (Entry, error) {
	meta := decodeRawMetadata(msg.Metadata)
	entry := Entry{
		EntryID:   stringField(meta, "entry_id"),
		EntryType: EntryType(stringField(meta, "entry_type")),
		RepoID:    stringField(meta, "repo_id"),
		Status:    Status(stringField(meta, "status")),
		Content:   msg.Content,
	}
	if entry.Status == "" {
		entry.Status = StatusLive
	}
	if ts := stringField(meta, "created_at"); ts != "" {
		if parsed, err := time.Parse(time.RFC3339Nano, ts); err == nil {
			entry.CreatedAt = parsed
		}
	}
	if entry.CreatedAt.IsZero() {
		entry.CreatedAt = msg.CreatedAt
	}
	entry.Scope = stringSliceField(meta, "scope")
	entry.EvidenceRefs = stringSliceField(meta, "evidence_refs")
	entry.RejectedAlternatives = altSliceField(meta, "rejected_alternatives")
	entry.TriggerEvidences = stringSliceField(meta, "trigger_evidences")
	entry.Constraints = stringSliceField(meta, "constraints")
	entry.Supersedes = stringField(meta, "supersedes")
	entry.SupersedeReason = stringField(meta, "supersede_reason")
	entry.Summary = stringField(meta, "summary")
	return entry, nil
}

func decodeEntries(msgs []honcho.Message) []Entry {
	out := make([]Entry, 0, len(msgs))
	for i := range msgs {
		if e, err := decodeEntry(&msgs[i]); err == nil && e.EntryID != "" {
			out = append(out, e)
		}
	}
	return out
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

func altSliceField(m map[string]any, key string) []Alternative {
	raw, ok := m[key]
	if !ok {
		return nil
	}
	arr, ok := raw.([]any)
	if !ok {
		return nil
	}
	out := make([]Alternative, 0, len(arr))
	for _, v := range arr {
		obj, ok := v.(map[string]any)
		if !ok {
			continue
		}
		out = append(out, Alternative{
			Text:   stringField(obj, "text"),
			Reason: stringField(obj, "reason"),
		})
	}
	return out
}

func entriesToHits(entries []Entry, _ []honcho.Message, baseScore float64) []RecallHit {
	hits := make([]RecallHit, 0, len(entries))
	for _, e := range entries {
		hits = append(hits, RecallHit{
			EntryID:   e.EntryID,
			EntryType: e.EntryType,
			Status:    e.Status,
			Content:   e.Content,
			Summary:   e.Summary,
			Scope:     e.Scope,
			Score:     baseScore,
			RepoID:    e.RepoID,
			CreatedAt: e.CreatedAt,
		})
	}
	sort.SliceStable(hits, func(i, j int) bool {
		return hits[i].CreatedAt.After(hits[j].CreatedAt)
	})
	return hits
}

func filterHits(hits []RecallHit, in RecallInput) []RecallHit {
	wantType := strings.TrimSpace(in.EntryType)
	prefix := strings.TrimSpace(in.ScopePrefix)
	out := make([]RecallHit, 0, len(hits))
	for _, h := range hits {
		if h.Status != "" && h.Status != StatusLive {
			continue
		}
		if wantType != "" {
			if string(h.EntryType) != wantType {
				continue
			}
		} else if h.EntryType == EntryHandoff && !in.IncludeHandoff {
			continue
		}
		if prefix != "" {
			matched := false
			for _, s := range h.Scope {
				if strings.HasPrefix(s, prefix) {
					matched = true
					break
				}
			}
			if !matched {
				continue
			}
		}
		out = append(out, h)
	}
	return out
}

func handoffResult(e Entry) *PickupHandoffResult {
	return &PickupHandoffResult{
		EntryID:   e.EntryID,
		Summary:   e.Summary,
		Content:   e.Content,
		CreatedAt: e.CreatedAt,
		RepoID:    e.RepoID,
	}
}

func cleanStrings(in []string) []string {
	out := make([]string, 0, len(in))
	for _, s := range in {
		s = strings.TrimSpace(s)
		if s != "" {
			out = append(out, s)
		}
	}
	if len(out) == 0 {
		return nil
	}
	return out
}

func cleanAlternatives(in []Alternative) []Alternative {
	out := make([]Alternative, 0, len(in))
	for _, a := range in {
		a.Text = strings.TrimSpace(a.Text)
		a.Reason = strings.TrimSpace(a.Reason)
		if a.Text != "" {
			out = append(out, a)
		}
	}
	if len(out) == 0 {
		return nil
	}
	return out
}
