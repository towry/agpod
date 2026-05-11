package mcpserver

import (
	"context"
	"encoding/json"
	"fmt"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/towry/agpod/internal/agpod-mcp/internal/memo"
)

// writeResult is the structured output for the three write tools.
type writeResult struct {
	EntryID string `json:"entry_id"`
}

type okResult struct {
	OK bool `json:"ok"`
}

// findingArgs mirrors memo.WriteFindingInput with explicit jsonschema tags so
// the MCP SDK can publish a usable input schema.
type findingArgs struct {
	Content      string   `json:"content" jsonschema:"factual statement worth preserving"`
	Scope        []string `json:"scope" jsonschema:"anchors — file:line, module path, or concept key — used to look this entry up later"`
	EvidenceRefs []string `json:"evidence_refs,omitempty" jsonschema:"optional commit hashes, PR urls, or doc paths"`
}

type decisionArgs struct {
	Content              string             `json:"content" jsonschema:"the chosen path"`
	Scope                []string           `json:"scope" jsonschema:"anchors this decision constrains"`
	RejectedAlternatives []memo.Alternative `json:"rejected_alternatives,omitempty"`
	TriggerEvidences     []string           `json:"trigger_evidences,omitempty"`
	Constraints          []string           `json:"constraints,omitempty"`
	EvidenceRefs         []string           `json:"evidence_refs,omitempty"`
	Supersedes           string             `json:"supersedes,omitempty" jsonschema:"entry_id of the prior decision this replaces"`
	SupersedeReason      string             `json:"supersede_reason,omitempty"`
}

type handoffArgs struct {
	Summary string `json:"summary" jsonschema:"short title shown when listing handoffs"`
	Content string `json:"content" jsonschema:"narrative body — progress, open questions, next steps"`
}

type pickupArgs struct {
	HandoffID string `json:"handoff_id,omitempty" jsonschema:"if set, fetch this specific handoff instead of the latest"`
	CrossRepo bool   `json:"cross_repo,omitempty"`
}

type recallArgs struct {
	Query          string `json:"query,omitempty"`
	EntryType      string `json:"entry_type,omitempty" jsonschema:"finding | decision | handoff"`
	ScopePrefix    string `json:"scope_prefix,omitempty"`
	IncludeHandoff bool   `json:"include_handoff,omitempty"`
	CrossRepo      bool   `json:"cross_repo,omitempty"`
	Limit          int    `json:"limit,omitempty"`
}

type whyArgs struct {
	Scope     string `json:"scope" jsonschema:"the anchor to look up — must match exactly one scope entry on a decision"`
	CrossRepo bool   `json:"cross_repo,omitempty"`
}

type setStatusArgs struct {
	EntryID string `json:"entry_id"`
	Status  string `json:"status" jsonschema:"superseded | no_longer_applicable"`
	Reason  string `json:"reason,omitempty"`
}

// registerTools attaches every memo_* tool to the server. When readonly is
// true, the four mutating tools (memo_write_* and memo_set_status) are
// omitted so the host never advertises them.
func registerTools(server *mcp.Server, store *memo.Store, readonly bool) {
	if !readonly {
		registerWriteTools(server, store)
	}
	registerReadTools(server, store)
}

func registerWriteTools(server *mcp.Server, store *memo.Store) {
	mcp.AddTool(server, &mcp.Tool{
		Name:        "memo_write_finding",
		Description: "Record an explored fact — how something works, where it lives, what convention applies — so future agents do not have to re-explore.",
	}, func(ctx context.Context, _ *mcp.CallToolRequest, args findingArgs) (*mcp.CallToolResult, *writeResult, error) {
		id, err := store.WriteFinding(ctx, memo.WriteFindingInput{
			Content:      args.Content,
			Scope:        args.Scope,
			EvidenceRefs: args.EvidenceRefs,
		})
		if err != nil {
			return nil, nil, err
		}
		return textResult(fmt.Sprintf("finding %s recorded", id)), &writeResult{EntryID: id}, nil
	})

	mcp.AddTool(server, &mcp.Tool{
		Name:        "memo_write_decision",
		Description: "Record a decision plus the rejected alternatives and constraints behind it. Pass `supersedes` to mark a previous decision as replaced.",
	}, func(ctx context.Context, _ *mcp.CallToolRequest, args decisionArgs) (*mcp.CallToolResult, *writeResult, error) {
		id, err := store.WriteDecision(ctx, memo.WriteDecisionInput{
			Content:              args.Content,
			Scope:                args.Scope,
			RejectedAlternatives: args.RejectedAlternatives,
			TriggerEvidences:     args.TriggerEvidences,
			Constraints:          args.Constraints,
			EvidenceRefs:         args.EvidenceRefs,
			Supersedes:           args.Supersedes,
			SupersedeReason:      args.SupersedeReason,
		})
		if err != nil {
			return nil, nil, err
		}
		return textResult(fmt.Sprintf("decision %s recorded", id)), &writeResult{EntryID: id}, nil
	})

	mcp.AddTool(server, &mcp.Tool{
		Name:        "memo_write_handoff",
		Description: "Snapshot the end of a session — progress, open questions, next steps — for the next agent to pick up.",
	}, func(ctx context.Context, _ *mcp.CallToolRequest, args handoffArgs) (*mcp.CallToolResult, *writeResult, error) {
		id, err := store.WriteHandoff(ctx, memo.WriteHandoffInput{
			Summary: args.Summary,
			Content: args.Content,
		})
		if err != nil {
			return nil, nil, err
		}
		return textResult(fmt.Sprintf("handoff %s recorded", id)), &writeResult{EntryID: id}, nil
	})

	mcp.AddTool(server, &mcp.Tool{
		Name:        "memo_set_status",
		Description: "Mark an entry as superseded or no_longer_applicable. Use sparingly — most entries stay live.",
	}, func(ctx context.Context, _ *mcp.CallToolRequest, args setStatusArgs) (*mcp.CallToolResult, *okResult, error) {
		if err := store.SetStatus(ctx, memo.SetStatusInput{
			EntryID: args.EntryID,
			Status:  memo.Status(args.Status),
			Reason:  args.Reason,
		}); err != nil {
			return nil, nil, err
		}
		return textResult(fmt.Sprintf("status of %s set to %s", args.EntryID, args.Status)), &okResult{OK: true}, nil
	})
}

func registerReadTools(server *mcp.Server, store *memo.Store) {
	mcp.AddTool(server, &mcp.Tool{
		Name:        "memo_pickup_handoff",
		Description: "Fetch the most recent live handoff for this repo (or a specific one by id).",
	}, func(ctx context.Context, _ *mcp.CallToolRequest, args pickupArgs) (*mcp.CallToolResult, *memo.PickupHandoffResult, error) {
		res, err := store.PickupHandoff(ctx, memo.PickupHandoffInput{
			HandoffID: args.HandoffID,
			CrossRepo: args.CrossRepo,
		})
		if err != nil {
			return nil, nil, err
		}
		return textResult(res.Summary + "\n\n" + res.Content), res, nil
	})

	mcp.AddTool(server, &mcp.Tool{
		Name:        "memo_recall",
		Description: "Search findings and decisions. Empty query lists recent entries. Pass include_handoff=true to also recall session handoffs.",
	}, func(ctx context.Context, _ *mcp.CallToolRequest, args recallArgs) (*mcp.CallToolResult, *recallResult, error) {
		hits, err := store.Recall(ctx, memo.RecallInput{
			Query:          args.Query,
			EntryType:      args.EntryType,
			ScopePrefix:    args.ScopePrefix,
			IncludeHandoff: args.IncludeHandoff,
			CrossRepo:      args.CrossRepo,
			Limit:          args.Limit,
		})
		if err != nil {
			return nil, nil, err
		}
		return jsonResult(hits), &recallResult{Hits: hits}, nil
	})

	mcp.AddTool(server, &mcp.Tool{
		Name:        "memo_why",
		Description: "Return the live decisions touching a scope anchor, each with its rejected alternatives and supersede chain.",
	}, func(ctx context.Context, _ *mcp.CallToolRequest, args whyArgs) (*mcp.CallToolResult, *memo.WhyResult, error) {
		res, err := store.Why(ctx, memo.WhyInput{Scope: args.Scope, CrossRepo: args.CrossRepo})
		if err != nil {
			return nil, nil, err
		}
		return jsonResult(res), res, nil
	})
}

type recallResult struct {
	Hits []memo.RecallHit `json:"hits"`
}

func textResult(text string) *mcp.CallToolResult {
	return &mcp.CallToolResult{
		Content: []mcp.Content{&mcp.TextContent{Text: text}},
	}
}

func jsonResult(v any) *mcp.CallToolResult {
	b, err := json.MarshalIndent(v, "", "  ")
	if err != nil {
		return textResult(fmt.Sprintf("<encode error: %v>", err))
	}
	return textResult(string(b))
}
