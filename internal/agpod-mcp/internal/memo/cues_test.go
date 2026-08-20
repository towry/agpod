package memo

import "testing"

func TestExtractTokens(t *testing.T) {
	got := ExtractTokens("agpod 用 `~/.config/agpod/nix-profile.sh` 补 HONCHO_WORKSPACE_ID 和 dots-orb-workflow")
	want := []string{
		"~/.config/agpod/nix-profile.sh",
		"HONCHO_WORKSPACE_ID",
		"dots-orb-workflow",
	}
	for _, w := range want {
		if !contains(got, w) {
			t.Fatalf("missing token %q in %v", w, got)
		}
	}
}

func TestCueOverlapFullCue(t *testing.T) {
	if cueOverlap("为什么 login shell 没有 nix", []string{"login shell 没有 nix"}) != 2 {
		t.Fatalf("expected full-cue overlap 2")
	}
}

func TestCueOverlapToken(t *testing.T) {
	if cueOverlap("nix-profile.sh missing in PATH", []string{"~/.config/agpod/nix-profile.sh"}) != 1 {
		t.Fatalf("expected token overlap 1")
	}
}

func TestStripAppendix(t *testing.T) {
	body := "fact here.\n\nfind: login shell 没有 nix | nix not on PATH"
	if got := stripAppendix(body); got != "fact here." {
		t.Fatalf("got %q", got)
	}
}

func TestMessageBodyOmitsAppendixWhenNoCues(t *testing.T) {
	if got := messageBody("fact", nil); got != "fact" {
		t.Fatalf("got %q", got)
	}
}

func contains(ss []string, want string) bool {
	for _, s := range ss {
		if s == want {
			return true
		}
	}
	return false
}

func TestSignificantWord(t *testing.T) {
	if significantWord("orb") {
		t.Fatal("orb is too short")
	}
	if significantWord("nix") {
		t.Fatal("nix is too short")
	}
	if !significantWord("toolchains") {
		t.Fatal("toolchains should count")
	}
	if !significantWord("重装") {
		t.Fatal("two CJK chars should count")
	}
	if significantWord("的") {
		t.Fatal("single CJK should not count")
	}
}

func TestCueOverlapIgnoresShortLatin(t *testing.T) {
	got := cueOverlap("does waking the orb reinstall toolchains", []string{"bootstrap-workflow.sh", "dots orb bootstrap"})
	if got == 2 {
		t.Fatalf("full cue overlap unexpected")
	}
	// "orb" alone must not produce token overlap against the bootstrap cue list
	// unless a significant token matches. bootstrap-workflow.sh does not appear
	// in the query, so overlap should be 0.
	if got != 0 {
		t.Fatalf("want 0, got %d", got)
	}
}

func TestCueCoveredWithFiller(t *testing.T) {
	if cueOverlap("login shell 为什么没有 nix", []string{"login shell 没有 nix"}) != 2 {
		t.Fatalf("query with filler words should still cover the cue")
	}
}

func TestCJKNgramOverlap(t *testing.T) {
	if contentOverlap("暂停后会不会重装依赖", "暂停后恢复不会重装依赖，resume 只重新挂 PATH。") == 0 {
		t.Fatal("same-language CJK paraphrase should overlap")
	}
	if contentOverlap("user's favorite pizza topping", "暂停后恢复不会重装依赖") != 0 {
		t.Fatal("unrelated English must not overlap CJK content")
	}
}

func TestPortIsSignificant(t *testing.T) {
	if !significantWord("6142") {
		t.Fatal("port numbers must count")
	}
	if cueOverlap("HTTP listen fails against localhost 8742", []string{"8742 被占用"}) == 0 {
		t.Fatal("8742 in query should overlap the port cue")
	}
}
