// Package memo defines the agent-memo data model and Honcho-backed store.
//
// Three entry types — finding, decision, handoff — share a single Entry shape
// whose semantics are discriminated by EntryType. All entries are bound to a
// single repo_id at the Honcho session level.
package memo

import "time"

type EntryType string

const (
	EntryFinding  EntryType = "finding"
	EntryDecision EntryType = "decision"
	EntryHandoff  EntryType = "handoff"
)

func (e EntryType) Valid() bool {
	switch e {
	case EntryFinding, EntryDecision, EntryHandoff:
		return true
	}
	return false
}

type Status string

const (
	StatusLive               Status = "live"
	StatusSuperseded         Status = "superseded"
	StatusNoLongerApplicable Status = "no_longer_applicable"
)

func (s Status) Valid() bool {
	switch s {
	case StatusLive, StatusSuperseded, StatusNoLongerApplicable:
		return true
	}
	return false
}

// Alternative is a rejected option considered while making a decision.
type Alternative struct {
	Text   string `json:"text"`
	Reason string `json:"reason,omitempty"`
}

// Entry is the unified record persisted as one Honcho message. Type-specific
// fields are populated only for their respective EntryType (e.g. Summary only
// for handoff). Empty slices and empty strings are dropped in the metadata
// payload to mirror the Rust adapter's `retain(|_, v| !v.is_null())` policy.
type Entry struct {
	EntryID   string    `json:"entry_id"`
	EntryType EntryType `json:"entry_type"`
	RepoID    string    `json:"repo_id"`
	Status    Status    `json:"status"`
	CreatedAt time.Time `json:"created_at"`

	// Body — semantically the finding/decision claim, or the handoff narrative.
	Content string `json:"content"`

	// Shared across finding/decision; ignored for handoff.
	Scope        []string `json:"scope,omitempty"`
	EvidenceRefs []string `json:"evidence_refs,omitempty"`

	// Decision-specific.
	RejectedAlternatives []Alternative `json:"rejected_alternatives,omitempty"`
	TriggerEvidences     []string      `json:"trigger_evidences,omitempty"`
	Constraints          []string      `json:"constraints,omitempty"`
	Supersedes           string        `json:"supersedes,omitempty"`
	SupersedeReason      string        `json:"supersede_reason,omitempty"`

	// Handoff-specific. Short title shown when listing.
	Summary string `json:"summary,omitempty"`
}

// WriteFindingInput is the payload for memo_write_finding.
type WriteFindingInput struct {
	Content      string   `json:"content"`
	Scope        []string `json:"scope"`
	EvidenceRefs []string `json:"evidence_refs,omitempty"`
}

// WriteDecisionInput is the payload for memo_write_decision.
type WriteDecisionInput struct {
	Content              string        `json:"content"`
	Scope                []string      `json:"scope"`
	RejectedAlternatives []Alternative `json:"rejected_alternatives,omitempty"`
	TriggerEvidences     []string      `json:"trigger_evidences,omitempty"`
	Constraints          []string      `json:"constraints,omitempty"`
	EvidenceRefs         []string      `json:"evidence_refs,omitempty"`
	Supersedes           string        `json:"supersedes,omitempty"`
	SupersedeReason      string        `json:"supersede_reason,omitempty"`
}

// WriteHandoffInput is the payload for memo_write_handoff.
type WriteHandoffInput struct {
	Summary string `json:"summary"`
	Content string `json:"content"`
}

// RecallInput is the payload for memo_recall.
type RecallInput struct {
	Query          string `json:"query,omitempty"`
	EntryType      string `json:"entry_type,omitempty"`
	ScopePrefix    string `json:"scope_prefix,omitempty"`
	IncludeHandoff bool   `json:"include_handoff,omitempty"`
	CrossRepo      bool   `json:"cross_repo,omitempty"`
	Limit          int    `json:"limit,omitempty"`
}

// RecallHit is a single recall result.
type RecallHit struct {
	EntryID   string    `json:"entry_id"`
	EntryType EntryType `json:"entry_type"`
	Status    Status    `json:"status"`
	Content   string    `json:"content"`
	Summary   string    `json:"summary,omitempty"`
	Scope     []string  `json:"scope,omitempty"`
	Score     float64   `json:"score"`
	RepoID    string    `json:"repo_id"`
	CreatedAt time.Time `json:"created_at"`
}

// PickupHandoffInput is the payload for memo_pickup_handoff.
type PickupHandoffInput struct {
	HandoffID string `json:"handoff_id,omitempty"`
	CrossRepo bool   `json:"cross_repo,omitempty"`
}

// PickupHandoffResult is the output for memo_pickup_handoff.
type PickupHandoffResult struct {
	EntryID   string    `json:"entry_id"`
	Summary   string    `json:"summary"`
	Content   string    `json:"content"`
	CreatedAt time.Time `json:"created_at"`
	RepoID    string    `json:"repo_id"`
}

// WhyInput is the payload for memo_why.
type WhyInput struct {
	Scope     string `json:"scope"`
	CrossRepo bool   `json:"cross_repo,omitempty"`
}

// SupersededLink represents one previous decision in a supersede chain.
type SupersededLink struct {
	EntryID         string    `json:"entry_id"`
	Content         string    `json:"content"`
	SupersedeReason string    `json:"supersede_reason,omitempty"`
	CreatedAt       time.Time `json:"created_at"`
}

// DecisionView is one decision plus its chain of predecessors.
type DecisionView struct {
	EntryID              string           `json:"entry_id"`
	Content              string           `json:"content"`
	Status               Status           `json:"status"`
	CreatedAt            time.Time        `json:"created_at"`
	Scope                []string         `json:"scope,omitempty"`
	RejectedAlternatives []Alternative    `json:"rejected_alternatives,omitempty"`
	TriggerEvidences     []string         `json:"trigger_evidences,omitempty"`
	Constraints          []string         `json:"constraints,omitempty"`
	EvidenceRefs         []string         `json:"evidence_refs,omitempty"`
	SupersedesChain      []SupersededLink `json:"supersedes_chain,omitempty"`
}

// WhyResult is the output of memo_why.
type WhyResult struct {
	Decisions []DecisionView `json:"decisions"`
}

// SetStatusInput is the payload for memo_set_status.
type SetStatusInput struct {
	EntryID string `json:"entry_id"`
	Status  Status `json:"status"`
	Reason  string `json:"reason,omitempty"`
}
