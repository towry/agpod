package memo

import (
	"regexp"
	"strings"
	"unicode"
)

var (
	pathLike  = regexp.MustCompile(`(?:~)?(?:\./|[\w.-]+/)+[\w.-]+`)
	envLike   = regexp.MustCompile(`\b[A-Z][A-Z0-9_]{2,}\b`)
	identLike = regexp.MustCompile(`\b[a-z][a-z0-9]*(?:-[a-z0-9]+)+\b`)
	backtick  = regexp.MustCompile("`([^`]+)`")
)

// ExtractTokens pulls search tokens from free text: paths, env names,
// hyphenated identifiers, and backtick spans. Used both at write time
// (merged into the message appendix) and at find time (cue overlap).
func ExtractTokens(text string) []string {
	seen := map[string]struct{}{}
	var out []string
	add := func(s string) {
		s = strings.TrimSpace(s)
		if s == "" {
			return
		}
		key := strings.ToLower(s)
		if _, ok := seen[key]; ok {
			return
		}
		seen[key] = struct{}{}
		out = append(out, s)
	}
	for _, m := range backtick.FindAllStringSubmatch(text, -1) {
		add(m[1])
	}
	for _, m := range pathLike.FindAllString(text, -1) {
		add(m)
	}
	for _, m := range envLike.FindAllString(text, -1) {
		add(m)
	}
	for _, m := range identLike.FindAllString(text, -1) {
		add(m)
	}
	return out
}

func cleanCues(in []string) []string {
	seen := map[string]struct{}{}
	var out []string
	for _, s := range in {
		s = collapseSpace(s)
		if s == "" {
			continue
		}
		key := strings.ToLower(s)
		if _, ok := seen[key]; ok {
			continue
		}
		seen[key] = struct{}{}
		out = append(out, s)
	}
	return out
}

func mergeCues(manual []string, auto []string) []string {
	return cleanCues(append(append([]string{}, manual...), auto...))
}

func collapseSpace(s string) string {
	return strings.Join(strings.Fields(strings.TrimSpace(s)), " ")
}

func normalizeKey(s string) string {
	return strings.ToLower(collapseSpace(s))
}

// cueOverlap reports how strongly query matches a cue set.
// 2 = a full cue is covered by the query (substring, or every cue word appears)
// 1 = an extracted identifier/path/env token or significant word overlaps
// 0 = none
func cueOverlap(query string, cues []string) int {
	q := normalizeKey(query)
	if q == "" {
		return 0
	}
	best := 0
	for _, c := range cues {
		ck := normalizeKey(c)
		if ck == "" {
			continue
		}
		if strings.Contains(q, ck) || strings.Contains(ck, q) || cueCovered(query, c) {
			if best < 2 {
				best = 2
			}
			continue
		}
	}
	if best == 2 {
		return 2
	}
	qTokens := map[string]struct{}{}
	for _, t := range ExtractTokens(query) {
		qTokens[strings.ToLower(t)] = struct{}{}
	}
	// also split query on non-letter so "login shell 没有 nix" hits cue "nix"
	for _, w := range splitWords(query) {
		qTokens[w] = struct{}{}
	}
	for _, c := range cues {
		for _, tok := range ExtractTokens(c) {
			if _, ok := qTokens[strings.ToLower(tok)]; ok {
				return 1
			}
		}
		for _, w := range splitWords(c) {
			if !significantWord(w) {
				continue
			}
			if _, ok := qTokens[w]; ok {
				return 1
			}
		}
	}
	return 0
}

func significantWord(w string) bool {
	rs := []rune(w)
	if len(rs) == 0 {
		return false
	}
	if n := cjkCount(w); n > 0 {
		return n >= 2
	}
	// Latin tokens shorter than this are magnets ("orb", "nix", "the").
	return len(rs) >= 6
}

func cueCovered(query, cue string) bool {
	cw := splitWords(cue)
	if len(cw) == 0 {
		return false
	}
	q := normalizeKey(query)
	qw := map[string]struct{}{}
	for _, w := range splitWords(query) {
		qw[w] = struct{}{}
	}
	for _, w := range cw {
		if _, ok := qw[w]; ok {
			continue
		}
		// CJK has no spaces, so "没有" must still match inside "为什么没有".
		if cjkCount(w) > 0 && strings.Contains(q, w) {
			continue
		}
		return false
	}
	if len(cw) >= 2 {
		return true
	}
	return significantWord(cw[0])
}

func cjkCount(w string) int {
	n := 0
	for _, r := range w {
		if r >= 0x4E00 && r <= 0x9FFF {
			n++
		}
	}
	return n
}

func splitWords(s string) []string {
	var b strings.Builder
	var out []string
	flush := func() {
		w := strings.ToLower(b.String())
		b.Reset()
		if w != "" {
			out = append(out, w)
		}
	}
	for _, r := range s {
		if unicode.IsLetter(r) || unicode.IsDigit(r) {
			b.WriteRune(r)
			continue
		}
		flush()
	}
	flush()
	return out
}

func messageBody(content string, cues []string) string {
	if len(cues) == 0 {
		return content
	}
	return content + "\n\n" + findPrefix + strings.Join(cues, " | ")
}

func stripAppendix(body string) string {
	if i := strings.Index(body, "\n\n"+findPrefix); i >= 0 {
		return strings.TrimSpace(body[:i])
	}
	if i := strings.Index(body, "\n"+findPrefix); i >= 0 {
		return strings.TrimSpace(body[:i])
	}
	return body
}

func contentOverlap(query, content string) int {
	return cueOverlap(query, append([]string{content}, ExtractTokens(content)...))
}
