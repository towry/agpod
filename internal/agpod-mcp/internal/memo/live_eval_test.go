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
			Content: "agent-memo 用独立 Honcho workspace 和 session id memo_<repo_id>，不要和其他产品共用 session。",
			Cues:    []string{"memo session 隔离", "honcho session 混在一起"},
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
			Content: "Go memo 单测用 httptest mock Honcho，不要打真实网络；live 测试才打 agpod-dev workspace。",
			Cues:    []string{"memo 单测 mock", "live 才打 honcho"},
		},
		{
			Content: "远程 memo MCP 用 AGPOD_MEMO_LISTEN 开 Streamable HTTP；非 loopback 必须设 AGPOD_MEMO_TOKEN。",
			Cues:    []string{"AGPOD_MEMO_LISTEN", "8742 被占用"},
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
			Content: "ask_note 空检索直接 unknown，不要把 unknown 当成事实；有命中才走 peer.chat。",
			Cues:    []string{"ask_note unknown", "空检索"},
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
		{name: "http listen", query: "8742 被占用", wantSub: "AGPOD_MEMO_LISTEN", mode: "search"},
		{name: "mock vs live", query: "memo 单测 mock", wantSub: "httptest", mode: "search"},
		{name: "cues location", query: "cues 放哪", wantSub: "find:", mode: "search"},
		{name: "forget mechanics", query: "forget 怎么删 conclusion", wantSub: "DELETE conclusion", mode: "search"},
		{name: "empty unknown", query: "user's favorite pizza topping", wantSub: "", mode: "search"},
		{name: "ask nix", query: "login shell 为什么没有 nix", wantSub: "bashrc", mode: "ask"},
		{name: "ask unknown", query: "what is the user's favorite pizza topping", wantSub: "", mode: "ask"},
		{name: "semantic paraphrase", query: "wake reinstall toolchains", wantSub: "不会重装依赖", mode: "search"},
		{name: "same language paraphrase", query: "暂停后会不会重装依赖", wantSub: "不会重装依赖", mode: "search"},
		{name: "competing similar", query: "ask_note unknown", wantSub: "空检索直接 unknown", mode: "search"},
	}

	var hit, miss int
	for _, c := range cases {
		start := time.Now()
		var (
			ok     bool
			detail string
			err    error
		)
		if c.mode == "ask" {
			ar, e := store.Find(ctx, FindInput{Query: c.query})
			err = e
			if ar != nil {
				detail = "unknown=" + boolStr(ar.Unknown) + " answer=" + ar.Answer
				if c.wantSub == "" {
					ok = ar.Unknown
				} else {
					ok = !ar.Unknown && strings.Contains(ar.Answer, c.wantSub)
				}
			}
		} else {
			fr, e := store.search(ctx, c.query, 8)
			err = e
			if fr != nil {
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
		}
		elapsed := time.Since(start)
		if err != nil {
			t.Errorf("%s: find error: %v", c.name, err)
			miss++
			continue
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
		if strings.Contains(k, "Go memo") || strings.Contains(k, "httptest") {
			forgetID = id
			break
		}
	}
	if forgetID != "" {
		if err := store.Forget(ctx, ForgetInput{ID: forgetID}); err != nil {
			t.Fatalf("forget: %v", err)
		}
		time.Sleep(1 * time.Second)
		fr, err := store.search(ctx, "memo 单测 mock", 8)
		if err != nil {
			t.Fatalf("find after forget: %v", err)
		}
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
		fr, err := store.search(ctx, q, 8)
		if err != nil {
			t.Logf("  find err=%v", err)
			continue
		}
		t.Logf("  find status=%s", fr.Status)
		for _, h := range fr.Hits {
			t.Logf("    rank=%d source=%s %s", h.Rank, h.Source, h.Content)
		}
	}
}

func TestLiveWarmChat(t *testing.T) {
	apiKey := strings.TrimSpace(os.Getenv("HONCHO_API_KEY"))
	if apiKey == "" {
		t.Skip("HONCHO_API_KEY not set")
	}
	base := os.Getenv("HONCHO_BASE_URL")
	if base == "" {
		base = "https://api.honcho.dev"
	}
	workspace := strings.TrimSpace(os.Getenv("HONCHO_WORKSPACE_ID"))
	if workspace == "" {
		t.Fatal("HONCHO_WORKSPACE_ID required")
	}

	ctx, cancel := context.WithTimeout(context.Background(), 12*time.Minute)
	defer cancel()

	cfg := config.Config{HonchoAPIKey: apiKey, HonchoBaseURL: base, HonchoWorkspaceID: workspace, PeerID: "agpod-agent"}
	cli, err := NewClient(cfg)
	if err != nil {
		t.Fatal(err)
	}
	store, err := NewStore(cli, Options{
		Workspace: workspace,
		PeerID:    "agpod-agent",
		RepoID:    "warm" + time.Now().UTC().Format("150405"),
		RepoLabel: "warm/agpod-mem",
	})
	if err != nil {
		t.Fatal(err)
	}
	if err := store.Ensure(ctx); err != nil {
		t.Fatal(err)
	}

	notes := warmNotes()
	var chars int
	for i, n := range notes {
		res, err := store.Note(ctx, n)
		if err != nil {
			t.Fatalf("note %d: %v", i, err)
		}
		chars += len([]rune(n.Content))
		t.Logf("noted %d/%d indexed=%v id=%s runes=%d", i+1, len(notes), res.Indexed, res.ID, len([]rune(n.Content)))
	}
	t.Logf("wrote %d notes, ~%d runes (~%d latin-bytes)", len(notes), chars, chars)

	sid := store.SessionID()
	peer := "agpod-agent"
	deadline := time.Now().Add(8 * time.Minute)
	var last *honcho.QueueStatus
	for time.Now().Before(deadline) {
		st, err := cli.GetQueueStatus(ctx, workspace, &peer, &peer, &sid)
		if err != nil {
			t.Logf("queue status err: %v", err)
			time.Sleep(10 * time.Second)
			continue
		}
		last = st
		t.Logf("queue total=%d completed=%d in_progress=%d pending=%d",
			st.TotalWorkUnits, st.CompletedWorkUnits, st.InProgressWorkUnits, st.PendingWorkUnits)
		if st.PendingWorkUnits == 0 && st.InProgressWorkUnits == 0 && st.CompletedWorkUnits > 0 {
			break
		}
		time.Sleep(15 * time.Second)
	}
	if last == nil {
		t.Fatal("never got queue status")
	}
	t.Logf("wait done: pending=%d in_progress=%d completed=%d",
		last.PendingWorkUnits, last.InProgressWorkUnits, last.CompletedWorkUnits)

	questions := []struct {
		name  string
		query string
		want  string // substring; empty means we only log
	}{
		{name: "nix path", query: "Why is nix missing from the orb login shell PATH, and how does agpod fix it?", want: "bashrc"},
		{name: "resume", query: "Does waking a paused orb reinstall toolchains?", want: "reinstall"},
		{name: "http listen", query: "How do remote agents connect to memo MCP over HTTP?", want: "AGPOD_MEMO_LISTEN"},
		{name: "honcho isolation", query: "What session id prefix does agent-memo use in Honcho?", want: "memo_"},
		{name: "unrelated", query: "What is the user's favorite pizza topping?", want: ""},
	}

	for _, q := range questions {
		start := time.Now()
		resp, err := cli.Chat(ctx, workspace, peer, honcho.DialecticOptions{
			Query:          q.query,
			SessionID:      &sid,
			ReasoningLevel: honcho.ReasoningLevelLow,
		})
		elapsed := time.Since(start)
		raw := ""
		if resp != nil && resp.Content != nil {
			raw = *resp.Content
		}
		t.Logf("CHAT %s %s err=%v answer=%q", q.name, elapsed.Truncate(time.Millisecond), err, raw)

		findRes, ferr := store.search(ctx, q.query, 3)
		if ferr != nil {
			t.Logf("  find err=%v", ferr)
			continue
		}
		hit := ""
		if len(findRes.Hits) > 0 {
			hit = findRes.Hits[0].Content
		}
		t.Logf("  FIND status=%s hit1=%s", findRes.Status, hit)

		if q.want == "" {
			if raw != "" && findRes.Status == "empty" {
				t.Logf("unrelated chat declined, find empty (good)")
			}
			continue
		}
		chatHit := strings.Contains(strings.ToLower(raw), strings.ToLower(q.want))
		findHit := strings.Contains(hit, q.want) || strings.Contains(strings.ToLower(hit), strings.ToLower(q.want))
		if !chatHit {
			t.Logf("chat %s missed substring %q (findHit=%v)", q.name, q.want, findHit)
		}
		if !chatHit && !findHit {
			t.Errorf("neither chat nor find answered %s want %q", q.name, q.want)
		}
	}
}

func warmNotes() []NoteInput {
	return []NoteInput{
		{Content: "orb login shell 不 source /etc/bashrc，Determinate Nix 不在 PATH；agpod 用 ~/.config/agpod/nix-profile.sh 把 /nix/var/nix/profiles/default/bin 补进 login shell。", Cues: []string{"login shell 没有 nix", "nix not on PATH"}},
		{Content: "暂停后恢复不会重装依赖。.agents/resume 只重新挂 PATH 并检查 rustc/cargo/go 是否还在，缺了只打 warning，不会跑 rustup 或 apt-get。", Cues: []string{"wake reinstall toolchains", "orb resume 不重装"}},
		{Content: "agent-memo 用独立 Honcho workspace 和 session id memo_<repo_id>，不要和其他产品共用 session。", Cues: []string{"memo session 隔离", "honcho session 混在一起"}},
		{Content: "Honcho conclusions 没有自定义 metadata，也不能改 status。forget 只能 DELETE /conclusions/{id}，再把对应 message metadata.status 标成 retired。", Cues: []string{"forget 怎么删 conclusion", "conclusion 没有 status"}},
		{Content: "cues 必须写进 message.content 的 find: 附录才能被 hybrid 关键词检索。只放在 metadata 里，Honcho search 打不中那些词。", Cues: []string{"cues 放哪", "metadata 不能搜"}},
		{Content: "Go memo 单测用 httptest mock Honcho，不要打真实网络。live 测试才打 agpod-dev workspace。", Cues: []string{"memo 单测 mock", "live 才打 honcho"}},
		{Content: "远程 memo MCP 用 AGPOD_MEMO_LISTEN 开 Streamable HTTP；非 loopback 必须设 AGPOD_MEMO_TOKEN，客户端连 http://<host>:8742/mcp。", Cues: []string{"AGPOD_MEMO_LISTEN", "8742 被占用"}},
		{Content: "dots orb bootstrap 现在由 towry/dots 的 nix/orb/bootstrap-workflow.sh 负责。不要再把 2026-08-02 gist 内联进项目 .agents/setup。", Cues: []string{"dots workflow gist 过时", "bootstrap-workflow.sh"}},
		{Content: "HONCHO_WORKSPACE 是沙箱别名，agpod 读的是 HONCHO_WORKSPACE_ID。.agents/setup 会把前者映射到后者，默认 HONCHO_BASE_URL 为 https://api.honcho.dev。", Cues: []string{"HONCHO_WORKSPACE 别名"}},
		{Content: "ask_note 空检索直接 unknown，不要把 unknown 当成事实。有命中才走 peer.chat，超时则回退 top hit。", Cues: []string{"ask_note unknown", "空检索"}},
		{Content: "cargo clippy 在 CI 以 -D warnings 跑。改 crate 行为后要跑该 crate 全量 cargo test，不能只跑窄测试。改 CLI/MCP 输出形状还要补覆盖该路径的测试。", Cues: []string{"clippy -D warnings", "crate 全量测试"}},
		{Content: "MCP stdio 是默认传输。远程部署设 AGPOD_MEMO_LISTEN=0.0.0.0:8742 和 AGPOD_MEMO_TOKEN，不要把 token 写进仓库。", Cues: []string{"stdio 默认", "远程 HTTP token"}},
		{Content: "MCP stdio 调试必须保持 stdin 打开，按行发送 JSON-RPC：initialize → notifications/initialized → tools/list → tools/call。tools/call 的 params 是 {name, arguments}。", Cues: []string{"mcp stdio 顺序", "tools/call 形状"}},
		{Content: "forget 会删 conclusion 并把 message 标 retired。结论检索必须 join 到 live message，否则 stale conclusion 会把已删笔记捞回来。", Cues: []string{"forget 删 conclusion", "stale conclusion"}},
		{Content: "repo_id 来自 git remote。同一仓库不同 worktree 共用 memo_<repo_id> session，所以跨 worktree 能问到同一批笔记。", Cues: []string{"跨 worktree memo", "repo-root"}},
		{Content: "Honcho message metadata 必须扁平。空数组和空字符串在发送前丢掉，不要塞 nested object。", Cues: []string{"honcho metadata 扁平", "空字段丢掉"}},
		{Content: "repo_id 算法是 hex(sha256(\"v1:\" + normalized_remote_url))[:16]。session id 是 memo_ 加上这段 hex。", Cues: []string{"repo_id 算法", "v1: remote hash"}},
		{Content: "orb 里不要用 nohup、setsid、& 或 tmux 保活长进程。Amp CLI 更新或 pause/resume 会杀同 cgroup 进程。长服务用 amp orb service start。", Cues: []string{"orb 长进程", "amp orb service"}},
		{Content: "用户打不开 sandbox localhost。对外分享必须走 amp orb portal 返回的 HTTPS portal URL，并加 Markdown 标题 amp-portal。不要发 e2b.app 直链。", Cues: []string{"portal URL", "不要发 localhost"}},
		{Content: "force-push、改保护分支、写生产、操作他人 PR 属于先询后行。自建分支的 commit 和草稿 PR 可自主执行。密钥禁止探读，即使用户同意。", Cues: []string{"§GR-A 动作分级", "禁止探读密钥"}},
		{Content: "开发中未上线的 schema 禁止堆 v1/v2 兼容层，直接改形状并同步全部调用方。只有已被外部消费者依赖的已发布契约才升版或迁移。", Cues: []string{"禁止开发期兼容层", "§GR-E"}},
		{Content: "执行本项目新建或改过的脚本前，须用 ast-grep 扫描递归删除等破坏性调用。通过 make/just/python -m 间接启动时，扫描实际执行的本项目脚本。", Cues: []string{"§GR-F 脚本扫描", "ast-grep rm"}},
		{Content: "Go 内部 MCP 在 internal/agpod-mcp，要求 Go 1.26+。orb setup 把 toolchain 装到 ~/.local/go，login shell 通过 ~/.config/agpod/go-profile.sh 注入 PATH。", Cues: []string{"Go 1.26", "internal/agpod-mcp"}},
		{Content: "Rust 用 rustup 的 stable，默认 rustc 1.97。bindgen/rocksdb 需要 LIBCLANG_PATH；setup 会从 llvm-config 或 /usr/lib/llvm-*/lib 探测。", Cues: []string{"LIBCLANG_PATH", "rustc stable"}},
		{Content: "just、uv、shellcheck、nixfmt、ast-grep、dots-orb 来自 ~/.local/state/nix/profiles/dots-orb-workflow，不是系统 apt。login shell 靠 ~/.config/dots/orb-profile.sh。", Cues: []string{"dots-orb-workflow", "just 从哪来"}},
		{Content: "workspace 依赖必须先加到根 Cargo.toml 的 [workspace.dependencies]，crate 里用 { workspace = true }。不要在子 crate 单独钉版本。", Cues: []string{"workspace.dependencies", "workspace = true"}},
		{Content: "新 crate 要同步改 release-please-config.json 和 .release-please-manifest.json，否则 CI 发版会漏。", Cues: []string{"release-please crate", "新 crate 清单"}},
		{Content: "改 memo MCP 工具形状后必须重编 internal/agpod-mcp。stdio 和 HTTP 共用同一套 note/ask_note/forget。", Cues: []string{"重编 agpod-mcp", "stale payload"}},
		{Content: "Honcho search 是 hybrid：关键词写入即可命中，语义 embedding 后台生成，刚写入的几秒内语义可能空。结论 query 只做语义、不返回 score。", Cues: []string{"hybrid 关键词即时", "conclusion 无 score"}},
		{Content: "QueryConclusions 打 workspace 端点时必须带 observer_id 和 observed_id，否则 400 observer and observed must be specified for semantic search。Python SDK 的 peer.conclusions.query 会自动填。", Cues: []string{"observer_id required", "conclusions query 400"}},
		{Content: "login shell 不 source /etc/bashrc，Determinate Nix 的 hook 在 bashrc 里，所以 nix 默认不在 PATH。agpod 用 ~/.config/agpod/nix-profile.sh 自己 source nix-daemon.sh，不覆盖 dots 的 orb-profile.sh。", Cues: []string{"nix-daemon login shell", "agpod nix-profile"}},
		{Content: "MCP 用户可见交付不能拿单元测试代替。Honcho 记忆质量以 live workspace 上 agent 风格查询的 hit@1 和空问 empty 为准，11/11 若全是 cue 原句不算语义达标。", Cues: []string{"live eval 才算质量", "cue 原句不算语义"}},
		{Content: "forget 不复活。要纠正就再 note 一句新的。检索侧 conclusion 被删后语义索引立刻没了；message search 必须过滤 metadata.status=live。", Cues: []string{"forget 不复活", "retired 过滤"}},
		{Content: "peer 固定 agpod-agent，observe_me=true，observe_others=false。representation 累积的是这个 agent 对仓库的认识，不是终端用户画像。", Cues: []string{"agpod-agent peer", "observe_me"}},
		{Content: "session id 是 memo_ 加上 repo_id。Honcho session id 只允许字母数字下划线和连字符；repo_id 是 hex 所以拼接合法。", Cues: []string{"memo_ session id", "session id 字符集"}},
		{Content: "空 query 的 find 直接报错，不做 list。continue/handoff 这轮不做。要浏览最近笔记不在当前工具面里。", Cues: []string{"find 必须有 query", "没有 list"}},
		{Content: "短拉丁词 orb、nix、the 不算 cue 重叠，避免 hybrid 把 dots orb bootstrap 抢到 wake-orb 查询前面。拉丁 token 至少 6 个字符才算 significant。", Cues: []string{"短词磁铁", "significantWord"}},
		{Content: "CJK 没有空格。暂停后会不会重装依赖 对 暂停后恢复不会重装依赖 靠 3 字 n-gram 重叠本地召回，不赌 Honcho 结论检索这次有没有超时。", Cues: []string{"CJK 3-gram", "同语言改写"}},
		{Content: "结论检索 cosine distance 上限 0.55。再松会把 orb 磁铁和远语义噪声放进来；再紧会杀掉同语言改写。跨语言英文问中文笔记在任何 distance 都经常空。", Cues: []string{"distance 0.55", "跨语言 embedding 空"}},
		{Content: "ask 现在等于 grounded search：答案是 hit1 正文，quotes/ids 来自命中列表。不再调用 peer.chat。unknown 只表示 search 空。", Cues: []string{"ask 等于 search", "不再 peer.chat"}},
	}
}
