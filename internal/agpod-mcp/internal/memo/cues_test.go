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
