// Package repoid derives a stable repository identity from the git remote URL.
// Algorithm mirrors crates/agpod-case/src/repo_id.rs so values match across tools.
//
// Keywords: repo-id, repository identity, git remote, normalize url
package repoid

import (
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"os/exec"
	"sort"
	"strings"
)

// Identity is a normalized repo identifier.
type Identity struct {
	RepoID    string // hex(sha256("v1:" + normalized))[0..16]
	RepoLabel string // e.g. "github.com/towry/agpod"
}

// ErrNotGitRepo is returned when the given path is not inside a git repo.
var ErrNotGitRepo = errors.New("not a git repository")

// ErrNoGitRemote is returned when the repo has no usable remote.
var ErrNoGitRemote = errors.New("no git remote configured")

// Resolve derives the identity from the given working directory.
func Resolve(cwd string) (Identity, error) {
	if err := checkGitDir(cwd); err != nil {
		return Identity{}, err
	}
	url, err := remoteURL(cwd)
	if err != nil {
		return Identity{}, err
	}
	normalized := NormalizeGitURL(url)
	return Identity{
		RepoID:    computeRepoID(normalized),
		RepoLabel: normalized,
	}, nil
}

func checkGitDir(cwd string) error {
	cmd := exec.Command("git", "rev-parse", "--git-dir")
	if cwd != "" {
		cmd.Dir = cwd
	}
	if err := cmd.Run(); err != nil {
		return ErrNotGitRepo
	}
	return nil
}

func remoteURL(cwd string) (string, error) {
	for _, name := range []string{"origin", "upstream"} {
		if url, ok := tryRemote(name, cwd); ok {
			return url, nil
		}
	}
	cmd := exec.Command("git", "remote")
	if cwd != "" {
		cmd.Dir = cwd
	}
	out, err := cmd.Output()
	if err != nil {
		return "", fmt.Errorf("list remotes: %w", err)
	}
	lines := strings.Split(strings.TrimSpace(string(out)), "\n")
	remotes := make([]string, 0, len(lines))
	for _, l := range lines {
		l = strings.TrimSpace(l)
		if l != "" {
			remotes = append(remotes, l)
		}
	}
	sort.Strings(remotes)
	if len(remotes) > 0 {
		if url, ok := tryRemote(remotes[0], cwd); ok {
			return url, nil
		}
	}
	return "", ErrNoGitRemote
}

func tryRemote(name, cwd string) (string, bool) {
	cmd := exec.Command("git", "remote", "get-url", name)
	if cwd != "" {
		cmd.Dir = cwd
	}
	out, err := cmd.Output()
	if err != nil {
		return "", false
	}
	url := strings.TrimSpace(string(out))
	if url == "" {
		return "", false
	}
	return url, true
}

// NormalizeGitURL collapses a git URL into "host/path" form (lowercased, no scheme/user, no `.git`).
func NormalizeGitURL(raw string) string {
	s := strings.TrimSpace(raw)

	// SSH shorthand: git@host:owner/repo.git
	if rest, ok := strings.CutPrefix(s, "git@"); ok {
		if i := strings.Index(rest, ":"); i >= 0 {
			return formatNormalized(rest[:i], rest[i+1:])
		}
	}

	// Protocol URLs
	for _, scheme := range []string{"ssh://", "https://", "http://"} {
		if rest, ok := strings.CutPrefix(s, scheme); ok {
			withoutUser := rest
			if at := strings.Index(rest, "@"); at >= 0 {
				withoutUser = rest[at+1:]
			}
			if slash := strings.Index(withoutUser, "/"); slash >= 0 {
				return formatNormalized(withoutUser[:slash], withoutUser[slash+1:])
			}
		}
	}

	return strings.ToLower(s)
}

func formatNormalized(host, path string) string {
	host = strings.ToLower(host)
	path = strings.TrimRight(path, "/")
	for strings.HasSuffix(path, ".git") {
		path = strings.TrimSuffix(path, ".git")
	}
	return host + "/" + strings.ToLower(path)
}

func computeRepoID(normalized string) string {
	sum := sha256.Sum256([]byte("v1:" + normalized))
	return hex.EncodeToString(sum[:8])
}
