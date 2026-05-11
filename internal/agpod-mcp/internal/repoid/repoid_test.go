package repoid

import "testing"

func TestNormalizeSSHShorthand(t *testing.T) {
	got := NormalizeGitURL("git@github.com:Org/Repo.git")
	if got != "github.com/org/repo" {
		t.Fatalf("want github.com/org/repo, got %s", got)
	}
}

func TestNormalizeHTTPS(t *testing.T) {
	got := NormalizeGitURL("https://github.com/Org/Repo.git")
	if got != "github.com/org/repo" {
		t.Fatalf("want github.com/org/repo, got %s", got)
	}
}

func TestNormalizeSSHProtocol(t *testing.T) {
	got := NormalizeGitURL("ssh://git@github.com/Org/Repo.git")
	if got != "github.com/org/repo" {
		t.Fatalf("want github.com/org/repo, got %s", got)
	}
}

func TestNormalizeWithoutGitSuffix(t *testing.T) {
	got := NormalizeGitURL("https://github.com/towry/agpod")
	if got != "github.com/towry/agpod" {
		t.Fatalf("got %s", got)
	}
}

func TestSSHAndHTTPSProduceSameID(t *testing.T) {
	a := NormalizeGitURL("git@github.com:towry/agpod.git")
	b := NormalizeGitURL("https://github.com/towry/agpod.git")
	if a != b {
		t.Fatalf("normalized mismatch: %q vs %q", a, b)
	}
	if computeRepoID(a) != computeRepoID(b) {
		t.Fatalf("repo_id mismatch")
	}
}

func TestRepoIDIs16HexChars(t *testing.T) {
	id := computeRepoID("github.com/towry/agpod")
	if len(id) != 16 {
		t.Fatalf("want length 16, got %d", len(id))
	}
	for _, c := range id {
		ok := (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f')
		if !ok {
			t.Fatalf("non-hex char %q in %s", c, id)
		}
	}
}

// TestRepoIDMatchesRustImplementation guards against drift from the Rust crate
// in crates/agpod-case/src/repo_id.rs. The expected value is the first 16 hex
// chars of sha256("v1:github.com/towry/agpod"), verified against
// `printf 'v1:github.com/towry/agpod' | shasum -a 256`.
func TestRepoIDMatchesRustImplementation(t *testing.T) {
	cases := []struct {
		normalized string
		want       string
	}{
		{"github.com/towry/agpod", "425a68e7d0ea73f1"},
	}
	for _, tc := range cases {
		got := computeRepoID(tc.normalized)
		if got != tc.want {
			t.Fatalf("normalized=%s want=%s got=%s", tc.normalized, tc.want, got)
		}
	}
}
