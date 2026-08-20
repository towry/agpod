// Package memo is the agent-memory store backed by Honcho v3.
//
// Agents persist short standalone facts with note, retrieve them with
// find, and retire them with forget. There is no kind/taxonomy: cues are
// optional short phrases the next agent is likely to search with.
package memo

import "time"

const (
	schemaV1      = "agpod.mem/1"
	statusLive    = "live"
	statusRetired = "retired"
	findPrefix    = "find: "
)

// NoteInput is the payload for note.
type NoteInput struct {
	Content string   `json:"content"`
	Cues    []string `json:"cues,omitempty"`
}

// NoteResult is returned by note.
type NoteResult struct {
	ID string `json:"id"`
}

// FindInput is the payload for find.
type FindInput struct {
	Query string `json:"query"`
	Mode  string `json:"mode,omitempty"` // search (default) | ask
	Limit int    `json:"limit,omitempty"`
}

// FindHit is one live memory surfaced by find.search.
type FindHit struct {
	ID        string    `json:"id,omitempty"`
	Content   string    `json:"content"`
	Cues      []string  `json:"cues,omitempty"`
	Source    string    `json:"source"`
	Rank      int       `json:"rank"`
	CreatedAt time.Time `json:"created_at"`
}

// FindResult is returned by find.search.
type FindResult struct {
	Status string    `json:"status"` // ok | empty
	Hits   []FindHit `json:"hits"`
}

// AskResult is returned by find.ask.
type AskResult struct {
	Answer  string   `json:"answer"`
	Unknown bool     `json:"unknown"`
	Quotes  []string `json:"quotes,omitempty"`
	IDs     []string `json:"ids,omitempty"`
}

// ForgetInput is the payload for forget.
type ForgetInput struct {
	ID string `json:"id"`
}

// entry is the canonical record persisted as one Honcho message.
type entry struct {
	EntryID      string
	RepoID       string
	Status       string
	Content      string // clean body, never the find: appendix
	Cues         []string
	ConclusionID string
	CreatedAt    time.Time
	MessageID    string
	SessionID    string
}
