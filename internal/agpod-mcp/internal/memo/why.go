package memo

import "sort"

// buildWhy reconstructs the decision graph for a scope query from a flat list
// of entries (all entry types are accepted; non-decisions are skipped).
//
// Returned decisions are status=live entries whose Scope contains the query.
// Each carries its full supersedes chain (oldest last) reconstructed by walking
// Supersedes pointers.
func buildWhy(entries []Entry, scope string) *WhyResult {
	byID := make(map[string]Entry, len(entries))
	for _, e := range entries {
		if e.EntryType == EntryDecision {
			byID[e.EntryID] = e
		}
	}

	views := make([]DecisionView, 0)
	for _, e := range entries {
		if e.EntryType != EntryDecision || e.Status != StatusLive {
			continue
		}
		if !scopeContains(e.Scope, scope) {
			continue
		}
		views = append(views, DecisionView{
			EntryID:              e.EntryID,
			Content:              e.Content,
			Status:               e.Status,
			CreatedAt:            e.CreatedAt,
			Scope:                e.Scope,
			RejectedAlternatives: e.RejectedAlternatives,
			TriggerEvidences:     e.TriggerEvidences,
			Constraints:          e.Constraints,
			EvidenceRefs:         e.EvidenceRefs,
			SupersedesChain:      chain(byID, e),
		})
	}

	sort.SliceStable(views, func(i, j int) bool {
		return views[i].CreatedAt.After(views[j].CreatedAt)
	})
	return &WhyResult{Decisions: views}
}

// chain walks backwards from `head` along Supersedes pointers. For each
// predecessor we record the SUCCEEDING decision's supersede_reason, since that
// is the rationale for retiring the predecessor.
func chain(byID map[string]Entry, head Entry) []SupersededLink {
	if head.Supersedes == "" {
		return nil
	}
	visited := make(map[string]struct{})
	var out []SupersededLink
	succeeding := head
	for cur := head.Supersedes; cur != ""; {
		if _, seen := visited[cur]; seen {
			break
		}
		visited[cur] = struct{}{}
		e, ok := byID[cur]
		if !ok {
			break
		}
		out = append(out, SupersededLink{
			EntryID:         e.EntryID,
			Content:         e.Content,
			SupersedeReason: succeeding.SupersedeReason,
			CreatedAt:       e.CreatedAt,
		})
		succeeding = e
		cur = e.Supersedes
	}
	return out
}

func scopeContains(scope []string, needle string) bool {
	for _, s := range scope {
		if s == needle {
			return true
		}
	}
	return false
}
