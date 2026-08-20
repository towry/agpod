package mcpserver

import (
	"context"
	"encoding/json"
	"fmt"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"github.com/towry/agpod/internal/agpod-mcp/internal/memo"
)

type noteArgs struct {
	Content string   `json:"content" jsonschema:"standalone present-tense fact that grep cannot recover"`
	Cues    []string `json:"cues,omitempty" jsonschema:"optional short phrases a later agent will type into find — 2 to 8 words, not categories"`
}

type findArgs struct {
	Query string `json:"query" jsonschema:"short search phrase like a cue, or a natural question"`
	Mode  string `json:"mode,omitempty" jsonschema:"search (default, ranked excerpts) or ask (synthesized answer)"`
	Limit int    `json:"limit,omitempty"`
}

type forgetArgs struct {
	ID string `json:"id" jsonschema:"id returned by note"`
}

type okResult struct {
	OK bool `json:"ok"`
}

func registerTools(server *mcp.Server, store *memo.Store, readonly bool) {
	if !readonly {
		registerWriteTools(server, store)
	}
	registerReadTools(server, store)
}

func registerWriteTools(server *mcp.Server, store *memo.Store) {
	mcp.AddTool(server, &mcp.Tool{
		Name: "note",
		Description: "Persist a fact for later sessions. Call as soon as you learn something grep cannot recover " +
			"(a convention, a pitfall, a choice with a real tradeoff). " +
			"content must stand alone in the present tense. " +
			"cues are optional short phrases you would type into find later; skip them when the content already contains the path, command, or env name. " +
			"Do not note file paths, signatures, or anything rg can answer.",
	}, func(ctx context.Context, _ *mcp.CallToolRequest, args noteArgs) (*mcp.CallToolResult, *memo.NoteResult, error) {
		res, err := store.Note(ctx, memo.NoteInput{Content: args.Content, Cues: args.Cues})
		if err != nil {
			return nil, nil, err
		}
		msg := fmt.Sprintf("noted %s", res.ID)
		if !res.Indexed {
			msg += " (keyword only; semantic index failed)"
		}
		return textResult(msg), res, nil
	})

	mcp.AddTool(server, &mcp.Tool{
		Name:        "forget",
		Description: "Retire a stored note that is wrong or no longer true. Pass the id returned by note. Does not resurrect.",
	}, func(ctx context.Context, _ *mcp.CallToolRequest, args forgetArgs) (*mcp.CallToolResult, *okResult, error) {
		if err := store.Forget(ctx, memo.ForgetInput{ID: args.ID}); err != nil {
			return nil, nil, err
		}
		return textResult(fmt.Sprintf("forgot %s", args.ID)), &okResult{OK: true}, nil
	})
}

func registerReadTools(server *mcp.Server, store *memo.Store) {
	mcp.AddTool(server, &mcp.Tool{
		Name: "find",
		Description: "Search stored notes before exploring. " +
			"query should look like a cue: a short concrete phrase (\"login shell 没有 nix\"), not \"any related memory\". " +
			"mode=search (default) returns ranked excerpts with ids. " +
			"mode=ask answers from stored notes. It searches first: unknown=true means nothing matched. If Honcho chat synthesizes, degraded is omitted; degraded=true means the answer is the top search hit because chat was empty.",
	}, func(ctx context.Context, _ *mcp.CallToolRequest, args findArgs) (*mcp.CallToolResult, any, error) {
		res, err := store.Find(ctx, memo.FindInput{Query: args.Query, Mode: args.Mode, Limit: args.Limit})
		if err != nil {
			return nil, nil, err
		}
		return jsonResult(res), res, nil
	})
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
