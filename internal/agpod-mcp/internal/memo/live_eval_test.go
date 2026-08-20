//go:build live

package memo

import (
	"context"
	"fmt"
	"os"
	"strings"
	"testing"
	"time"

	honcho "github.com/hekmon/go-honcho"

	"github.com/towry/agpod/internal/agpod-mcp/internal/config"
)

type evalCase struct {
	name    string
	query   string
	wantSub string // substring of expected content; empty means want empty/unknown
	mode    string
}

func TestLiveAgentScenarios(t *testing.T) {
	apiKey := strings.TrimSpace(os.Getenv("HONCHO_API_KEY"))
	if apiKey == "" {
		t.Skip("HONCHO_API_KEY not set")
	}
	base := os.Getenv("HONCHO_BASE_URL")
	if base == "" {
		base = "https://api.honcho.dev"
	}
	workspace := strings.TrimSpace(os.Getenv("AGPOD_MEMO_EVAL_WORKSPACE"))
	if workspace == "" {
		workspace = strings.TrimSpace(os.Getenv("HONCHO_WORKSPACE_ID"))
	}
	if workspace == "" {
		t.Fatal("set AGPOD_MEMO_EVAL_WORKSPACE or HONCHO_WORKSPACE_ID")
	}

	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Minute)
	defer cancel()

	cfg := config.Config{HonchoAPIKey: apiKey, HonchoBaseURL: base, HonchoWorkspaceID: workspace, PeerID: "agpod-agent"}
	cli, err := NewClient(cfg)
	if err != nil {
		t.Fatal(err)
	}
	store, err := NewStore(cli, Options{
		Workspace: workspace,
		PeerID:    "agpod-agent",
		RepoID:    "eval" + time.Now().UTC().Format("150405"),
		RepoLabel: "eval/agpod-mem",
	})
	if err != nil {
		t.Fatal(err)
	}
	if err := store.Ensure(ctx); err != nil {
		t.Fatalf("ensure: %v", err)
	}

	notes := []NoteInput{
		{
			Content: "orb login shell 不 source /etc/bashrc，Determinate Nix 不在 PATH；agpod 用 ~/.config/agpod/nix-profile.sh 补。",
			Cues:    []string{"login shell 没有 nix", "nix not on PATH"},
		},
		{
			Content: "agent-memo 与 agpod-case 共用 Honcho 时必须分 session：memo 用 memo_<repo_id>，case 用 case id，find 默认不搜 case 事件。",
			Cues:    []string{"memo session 隔离", "case honcho 混在一起"},
		},
		{
			Content: "Honcho conclusions 没有自定义 metadata，也不能改 status；forget 只能 DELETE conclusion，再把 message metadata.status 标 retired。",
			Cues:    []string{"forget 怎么删 conclusion", "conclusion 没有 status"},
		},
		{
			Content: "cues 必须写进 message.content 的 find: 附录才能被 hybrid 关键词检索；只放 metadata 搜不到。",
			Cues:    []string{"cues 放哪", "metadata 不能搜"},
		},
		{
			Content: "SurrealDB case store 在测试里不能用 RocksDB：并行测试会挂，改用 mem backend。",
			Cues:    []string{"case db 测试挂", "rocksdb hang tests"},
		},
		{
			Content: "agpod-mcp 的 Go 二进制会自动拉起 sibling agpod-case-server；MCP smoke 前要先杀掉占用 127.0.0.1:6142 的旧进程。",
			Cues:    []string{"stale case-server", "6142 被占用"},
		},
		{
			Content: "dots orb bootstrap 现在由 towry/dots 的 nix/orb/bootstrap-workflow.sh 负责，不要再内联 8 月 gist。",
			Cues:    []string{"dots workflow gist 过时", "bootstrap-workflow.sh"},
		},
		{
			Content: "HONCHO_WORKSPACE 是沙箱别名，agpod 读的是 HONCHO_WORKSPACE_ID；setup 会做映射。",
			Cues:    []string{"HONCHO_WORKSPACE 别名"},
		},
		{
			Content: "暂停后恢复不会重装依赖，resume 只重新挂 PATH。",
			Cues:    []string{"wake reinstall toolchains", "orb resume 不重装"},
		},
		{
			Content: "case_open 失败时不要重试同一 payload，先查 6142 是否被旧 agpod-case-server 占用。",
			Cues:    []string{"case_open 失败"},
		},
	}

	ids := map[string]string{} // content snippet -> id
	for _, n := range notes {
		res, err := store.Note(ctx, n)
		if err != nil {
			t.Fatalf("note %q: %v", n.Content[:40], err)
		}
		ids[n.Content[:20]] = res.ID
		t.Logf("noted %s  %s", res.ID, n.Content[:40])
	}

	// Keyword search is immediate; still give conclusions a brief moment.
	time.Sleep(2 * time.Second)

	cases := []evalCase{
		{name: "cue exact zh", query: "login shell 没有 nix", wantSub: "Determinate Nix", mode: "search"},
		{name: "agent oral zh", query: "为什么 orb 里 nix 不在 PATH", wantSub: "nix-profile.sh", mode: "search"},
		{name: "cue exact en", query: "nix not on PATH", wantSub: "Determinate Nix", mode: "search"},
		{name: "identifier", query: "bootstrap-workflow.sh", wantSub: "towry/dots", mode: "search"},
		{name: "port conflict", query: "6142 被占用", wantSub: "agpod-case-server", mode: "search"},
		{name: "confusable db", query: "case db 测试挂", wantSub: "mem backend", mode: "search"},
		{name: "cues location", query: "cues 放哪", wantSub: "find:", mode: "search"},
		{name: "forget mechanics", query: "forget 怎么删 conclusion", wantSub: "DELETE conclusion", mode: "search"},
		{name: "empty unknown", query: "user's favorite pizza topping", wantSub: "", mode: "search"},
		{name: "ask nix", query: "login shell 为什么没有 nix", wantSub: "bashrc", mode: "ask"},
		{name: "ask unknown", query: "what is the user's favorite pizza topping", wantSub: "", mode: "ask"},
		{name: "semantic paraphrase", query: "wake reinstall toolchains", wantSub: "不会重装依赖", mode: "search"},
		{name: "same language paraphrase", query: "暂停后会不会重装依赖", wantSub: "不会重装依赖", mode: "search"},
		{name: "competing similar", query: "case_open 失败", wantSub: "不要重试同一 payload", mode: "search"},
	}

	var hit, miss int
	for _, c := range cases {
		start := time.Now()
		res, err := store.Find(ctx, FindInput{Query: c.query, Mode: c.mode, Limit: 8})
		elapsed := time.Since(start)
		if err != nil {
			t.Errorf("%s: find error: %v", c.name, err)
			miss++
			continue
		}
		ok := false
		detail := ""
		switch c.mode {
		case "ask":
			ar := res.(*AskResult)
			detail = "unknown=" + boolStr(ar.Unknown) + " degraded=" + boolStr(ar.Degraded) + " answer=" + ar.Answer
			if c.wantSub == "" {
				ok = ar.Unknown
			} else {
				ok = !ar.Unknown && strings.Contains(ar.Answer, c.wantSub)
			}
		default:
			fr := res.(*FindResult)
			if len(fr.Hits) > 0 {
				detail = "hit1=" + fr.Hits[0].Content
			} else {
				detail = "empty"
			}
			if c.wantSub == "" {
				ok = fr.Status == "empty" || len(fr.Hits) == 0
			} else if len(fr.Hits) > 0 {
				ok = strings.Contains(fr.Hits[0].Content, c.wantSub)
			}
		}
		if ok {
			hit++
			t.Logf("PASS %s  %s  %s", c.name, elapsed.Truncate(time.Millisecond), detail)
		} else {
			miss++
			t.Errorf("FAIL %s query=%q wantSub=%q  %s  %s", c.name, c.query, c.wantSub, elapsed.Truncate(time.Millisecond), detail)
		}
	}
	t.Logf("score %d/%d", hit, hit+miss)
	if miss > 0 {
		t.Fatalf("%d cases failed", miss)
	}

	// forget then confirm the retired note is gone from live search
	var forgetID string
	for k, id := range ids {
		if strings.Contains(k, "SurrealDB") || strings.HasPrefix(k, "SurrealDB") {
			forgetID = id
		}
	}
	if forgetID == "" {
		// ids keyed by first 20 runes of content
		for k, id := range ids {
			if strings.Contains(k, "Surreal") {
				forgetID = id
				break
			}
		}
	}
	if forgetID != "" {
		if err := store.Forget(ctx, ForgetInput{ID: forgetID}); err != nil {
			t.Fatalf("forget: %v", err)
		}
		time.Sleep(1 * time.Second)
		res, err := store.Find(ctx, FindInput{Query: "case db 测试挂", Mode: "search"})
		if err != nil {
			t.Fatalf("find after forget: %v", err)
		}
		fr := res.(*FindResult)
		for _, h := range fr.Hits {
			if h.ID == forgetID {
				t.Fatalf("forgotten id still in live hits")
			}
		}
	}
}

func boolStr(v bool) string {
	if v {
		return "true"
	}
	return "false"
}

func TestLiveSemanticProbe(t *testing.T) {
	apiKey := strings.TrimSpace(os.Getenv("HONCHO_API_KEY"))
	if apiKey == "" {
		t.Skip("HONCHO_API_KEY not set")
	}
	base := os.Getenv("HONCHO_BASE_URL")
	if base == "" {
		base = "https://api.honcho.dev"
	}
	workspace := strings.TrimSpace(os.Getenv("HONCHO_WORKSPACE_ID"))
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Minute)
	defer cancel()
	cfg := config.Config{HonchoAPIKey: apiKey, HonchoBaseURL: base, HonchoWorkspaceID: workspace, PeerID: "agpod-agent"}
	cli, err := NewClient(cfg)
	if err != nil {
		t.Fatal(err)
	}
	store, err := NewStore(cli, Options{Workspace: workspace, PeerID: "agpod-agent", RepoID: "probe" + time.Now().UTC().Format("150405"), RepoLabel: "probe"})
	if err != nil {
		t.Fatal(err)
	}
	if err := store.Ensure(ctx); err != nil {
		t.Fatal(err)
	}
	notes := []NoteInput{
		{Content: "暂停后恢复不会重装依赖，resume 只重新挂 PATH。", Cues: []string{"wake reinstall toolchains", "orb resume 不重装"}},
		{Content: "dots orb bootstrap 现在由 towry/dots 的 nix/orb/bootstrap-workflow.sh 负责，不要再内联 8 月 gist。", Cues: []string{"bootstrap-workflow.sh"}},
		{Content: "orb login shell 不 source /etc/bashrc，Determinate Nix 不在 PATH。", Cues: []string{"login shell 没有 nix"}},
	}
	for _, n := range notes {
		res, err := store.Note(ctx, n)
		if err != nil {
			t.Fatal(err)
		}
		t.Logf("noted indexed=%v %s", res.Indexed, n.Content)
	}
	time.Sleep(8 * time.Second)
	sid := store.SessionID()
	d55, d70, d80, d90 := 0.55, 0.70, 0.80, 0.90
	queries := []string{
		"does waking the sandbox reinstall toolchains",
		"does waking the orb reinstall toolchains",
		"user's favorite pizza topping",
		"暂停后会不会重装依赖",
	}
	for _, q := range queries {
		t.Logf("QUERY %s", q)
		for _, dist := range []*float64{&d55, &d70, &d80, &d90, nil} {
			req := honcho.ConclusionQuery{
				Query:    q,
				TopK:     5,
				Distance: dist,
				Filters: map[string]any{
					"session_id":  sid,
					"observer_id": "agpod-agent",
					"observed_id": "agpod-agent",
					"level":       "explicit",
				},
			}
			concs, err := cli.QueryConclusions(ctx, workspace, req)
			label := "no-dist"
			if dist != nil {
				label = fmt.Sprintf("d=%.2f", *dist)
			}
			if err != nil {
				t.Logf("  %s err=%v", label, err)
				continue
			}
			if len(concs) == 0 {
				t.Logf("  %s (none)", label)
				continue
			}
			for i, c := range concs {
				if c != nil {
					t.Logf("  %s %d. %s", label, i+1, c.Content)
				}
			}
		}
		res, err := store.Find(ctx, FindInput{Query: q, Mode: "search", Limit: 8})
		if err != nil {
			t.Logf("  find err=%v", err)
			continue
		}
		fr := res.(*FindResult)
		t.Logf("  find status=%s", fr.Status)
		for _, h := range fr.Hits {
			t.Logf("    rank=%d source=%s %s", h.Rank, h.Source, h.Content)
		}
	}
}
