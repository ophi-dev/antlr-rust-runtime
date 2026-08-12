// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
use super::*;

/// Context for translating one action/predicate body.
pub(crate) struct TranslationCtx<'a> {
    pub(crate) model: &'a EmbeddedModel,
    /// Rule containing the body.
    pub(crate) rule_index: usize,
    /// Byte offset of the body inside the grammar source, used to pick the
    /// enclosing alternative for label resolution. `None` for `@init` /
    /// `@after` bodies (labels resolve across all alternatives there).
    pub(crate) body_offset: Option<usize>,
    pub(crate) site: ActionSite,
    /// Token name -> token type, from the compiled recognizer metadata.
    pub(crate) token_types: &'a BTreeMap<String, i32>,
}

impl TranslationCtx<'_> {
    fn rule(&self) -> &RuleModel {
        &self.model.rules[self.rule_index]
    }

    fn rule_index_by_name(&self, name: &str) -> Option<usize> {
        self.model.rules.iter().position(|rule| rule.name == name)
    }

    /// Resolves a label to `(ref, occurrence-among-same-target-in-alt)`.
    ///
    /// Every read `translate_element_read` can emit is a *positional* query over
    /// the flattened CST children — `nth(i)` for a single label, "all children of
    /// this target" for a list label, "the last terminal child" for a block
    /// label. None of those retain which grammar branch built a child, so a
    /// label only resolves when its read provably selects the label's own
    /// element and nothing else. When it cannot, this returns `None` and the
    /// caller fails loudly rather than translating to a silently wrong read.
    ///
    /// The conditions that make a read unfaithful, by label kind:
    ///
    /// * **single** — a preceding ref with inexact cardinality (sibling branches
    ///   of a choice are mutually exclusive and report `min: 0`, so counting
    ///   them indexes past what the parse built), or an *optional* label with a
    ///   following same-target child that slides into its position when absent;
    /// * **list** — any same-target child outside the label, since the read
    ///   cannot exclude it;
    /// * **block** — a following terminal, which would become the `last()` the
    ///   read takes;
    /// * **any kind** — a second declaration of the same label that this one
    ///   read cannot also serve.
    fn resolve_label(&self, label: &str) -> Option<(ElementRef, usize)> {
        let rule = self.rule();
        if self.site == ActionSite::Init {
            // An `@init` body runs at rule entry, before any child exists, so every
            // read over children is empty and nothing can pollute it — no hazard
            // applies. Most reads degrade gracefully (an iterator yields nothing, a
            // `.text` read yields `""`), but a *scalar rule* label lowers to
            // `.nth(i).expect("labeled rule child")`, which panics on every parse.
            // Decline that one rather than emit code that cannot run.
            let element = rule
                .alts
                .iter()
                .flat_map(|alt| alt.refs.iter())
                .find(|element| element.label.as_deref() == Some(label))?;
            let panics_when_absent =
                !element.is_list && element.token_types.is_empty() && !element.target.is_empty();
            return (!panics_when_absent).then(|| (element.clone(), 0));
        }
        if let Some((offset, alt)) = self
            .body_offset
            .and_then(|offset| rule.alt_at(offset).map(|alt| (offset, alt)))
        {
            // A mid-rule action executes at its own source position, so only refs
            // starting before it have been matched, and only branches enclosing
            // that position can have run.
            return Self::resolve_label_in_alt(alt, label, Some(offset));
        }
        // `@after` / `@init` bodies are not scoped to an alternative, so the
        // label may be declared in several. One read has to serve whichever
        // alternative the parse took: taking the first match would emit that
        // alternative's lookup and silently yield a default on the others.
        let mut resolved: Option<(ElementRef, usize)> = None;
        let mut non_declaring = Vec::new();
        for alt in &rule.alts {
            let declares = alt
                .refs
                .iter()
                .any(|element| element.label.as_deref() == Some(label));
            if !declares {
                non_declaring.push(alt);
                continue;
            }
            // An `@after` / `@init` body runs whichever branch the parse took, so
            // a sibling branch's match *can* be the child present when the read
            // executes — sibling exclusion does not apply here.
            let candidate = Self::resolve_label_in_alt(alt, label, None)?;
            if resolved
                .as_ref()
                .is_some_and(|existing| !Self::same_label_read(existing, &candidate))
            {
                return None;
            }
            resolved = Some(candidate);
        }
        let (element, occurrence) = resolved?;
        // An alternative that never declares the label leaves it unset, so the
        // read must come up empty there. It will not if that alternative happens
        // to build a child the read would select anyway (`r : x=A | A`), which
        // would report a value for a label the parse never bound.
        for alt in non_declaring {
            if Self::alt_can_satisfy_read(alt, &element, occurrence) {
                return None;
            }
        }
        Some((element, occurrence))
    }

    /// Whether `alt` builds a child that the read for `element` would select,
    /// even though `alt` does not declare the label.
    fn alt_can_satisfy_read(alt: &AltModel, element: &ElementRef, occurrence: usize) -> bool {
        // Route on `is_block`, matching `translate_element_read`: a *literal* label
        // (`x='a'`) is block-mode yet keeps a non-empty source target, so keying on
        // an empty target here would fall through to token-type matching while the
        // read actually ignores token type entirely.
        if element.is_block {
            // A positional block read selects a terminal by index, so the
            // alternative can satisfy it whenever it builds a terminal there.
            return alt
                .refs
                .iter()
                .filter(|candidate| {
                    !candidate.token_types.is_empty() && candidate.cardinality.max != Some(0)
                })
                .any(|candidate| Self::can_occupy_terminal_index(alt, candidate, occurrence));
        }
        // The read queries by token *type*, so a differently-spelled terminal with
        // the same type is the same child (`A : 'a';` makes `A` and `'a'` one).
        let same_read_target = |candidate: &ElementRef| {
            if element.token_types.is_empty() || candidate.token_types.is_empty() {
                return candidate.target == element.target;
            }
            candidate
                .token_types
                .iter()
                .any(|token_type| element.token_types.contains(token_type))
        };
        // The most matching children *one parse* can build. Sequential refs add;
        // branches of a choice are alternatives, so the widest branch wins. Nested
        // choices must fold innermost-first — reducing each choice independently
        // and then summing would double-count, rejecting valid reads such as
        // `q x=q | ((q|b)|(q|c))` where the second alternative builds one `q`.
        let available = Self::widest_child_count(
            alt.refs
                .iter()
                .filter(|candidate| same_read_target(candidate)),
        );
        // A list read selects any same-target child; a positional read needs one
        // at `occurrence`. An unbounded count can always reach either.
        available.is_none_or(|available| available > occurrence)
    }

    /// Whether `candidate` can occupy terminal index `occurrence` on its own parse
    /// path. A *repeated* candidate spans a range of positions rather than one, so
    /// comparing a single index would miss it: in `C x=(A | B) | (D | E)+` the
    /// repeated group starts at 0 yet also covers 1, where `x` reads.
    fn can_occupy_terminal_index(
        alt: &AltModel,
        candidate: &ElementRef,
        occurrence: usize,
    ) -> bool {
        // `usize::MAX` is the sentinel for "no fixed index, the read falls back to
        // `last()`" — not a position. Any terminal the alternative builds can be that
        // last child, so every candidate can occupy it.
        if occurrence == usize::MAX {
            return true;
        }
        let Some(start) = Self::exact_terminal_index(alt, candidate) else {
            // No fixed start: the candidate could be anywhere.
            return true;
        };
        if start > occurrence {
            return false;
        }
        // Unbounded repetition reaches every later index.
        candidate
            .cardinality
            .max
            .is_none_or(|max| start + max > occurrence)
    }

    /// Index among terminal children at which `element` sits on its own parse
    /// path, or `None` when that index is not fixed.
    fn exact_terminal_index(alt: &AltModel, element: &ElementRef) -> Option<usize> {
        let position = alt
            .refs
            .iter()
            .position(|candidate| std::ptr::eq(candidate, element))?;
        // `can_coexist_with` keeps everything a ref *after* the choice coexists
        // with — both branches — so it does not by itself select one path. Strip the
        // tags of choices the element is inside (those branches are taken on its
        // path) and leave the rest tagged, so `exact_child_count` still demands
        // cross-branch agreement for choices the element is not part of.
        let on_path = alt.refs[..position]
            .iter()
            .filter(|candidate| {
                !candidate.token_types.is_empty() && candidate.can_coexist_with(element)
            })
            .cloned()
            .map(|mut candidate| {
                candidate.retain_choices(|choice| {
                    !element
                        .choice_branch
                        .iter()
                        .any(|&(taken, _)| taken == choice)
                });
                if candidate.choice_branch.is_empty() {
                    candidate.cardinality = candidate.group_local_cardinality;
                }
                candidate
            })
            .collect::<Vec<_>>();
        Self::exact_child_count(on_path.iter(), false)
    }

    /// Total children contributed by `refs`, or `None` when that total is not the
    /// same on every parse.
    ///
    /// Refs are grouped by their enclosing choices and folded **innermost-first**:
    /// once a choice's branches agree, its count is attributed to the enclosing
    /// branch that contains it, so nested exhaustive choices
    /// (`((a=A | b=A) | c=A)`) stay exact while nested *differing* ones
    /// (`((a=A | b=B) C | D)`) correctly do not.
    ///
    /// `restricted_to_one_path` says the caller already filtered `refs` down to a
    /// single parse path. Cross-branch agreement is then meaningless — the other
    /// branches were removed on purpose — so each surviving branch simply counts.
    fn exact_child_count<'a>(
        refs: impl Iterator<Item = &'a ElementRef>,
        restricted_to_one_path: bool,
    ) -> Option<usize> {
        let refs = refs.collect::<Vec<_>>();
        let mut total = 0_usize;
        let mut per_branch: BTreeMap<(usize, usize), Option<usize>> = BTreeMap::new();
        // Arity is recorded, not observed: an *empty* alternative emits no ref, so
        // `(a=A | )` would otherwise look like a one-branch choice.
        let mut arity_of_choice: BTreeMap<usize, usize> = BTreeMap::new();
        let mut depth_of_choice: BTreeMap<usize, usize> = BTreeMap::new();
        let mut ancestry: BTreeMap<(usize, usize), Vec<(usize, usize)>> = BTreeMap::new();
        for candidate in &refs {
            for (depth, (&(choice, branch), &arity)) in candidate
                .choice_branch
                .iter()
                .zip(&candidate.choice_arity)
                .enumerate()
            {
                arity_of_choice.insert(choice, arity);
                // A choice's depth is where it sits in the ancestry; take the
                // *shallowest* sighting, since that is its real nesting level
                // (a deeper ref lists it at the same index, never a lower one).
                depth_of_choice
                    .entry(choice)
                    .and_modify(|existing| *existing = (*existing).min(depth))
                    .or_insert(depth);
                ancestry.insert((choice, branch), candidate.choice_branch[..=depth].to_vec());
            }
        }
        for candidate in &refs {
            let max = candidate.cardinality.max?;
            // Within its own branch a ref contributes its branch-local count; the
            // `min: 0` that branch membership imposes is not optionality.
            let local = candidate.branch_local_cardinality;
            let exact = (local.min == max && local.max == Some(max)).then_some(max);
            match candidate.choice_branch.last() {
                None => total = total.saturating_add(exact?),
                Some(&key) => {
                    let slot = per_branch.entry(key).or_insert(Some(0));
                    *slot = match (*slot, exact) {
                        (Some(sum), Some(next)) => Some(sum.saturating_add(next)),
                        _ => None,
                    };
                }
            }
        }
        // Deepest choices first, so an inner result rolls up into its parent branch.
        // The list is rebuilt from `per_branch` each pass, because folding an inner
        // choice *creates* an entry for its parent that must then fold in turn.
        let mut processed: BTreeSet<usize> = BTreeSet::new();
        // Deepest unprocessed choice still holding entries. Folding one creates an
        // entry for its parent, so the candidate set is re-examined every pass.
        while let Some(choice) = per_branch
            .keys()
            .map(|(choice, _)| *choice)
            .filter(|choice| !processed.contains(choice))
            .max_by_key(|choice| depth_of_choice.get(choice).copied().unwrap_or(0))
        {
            processed.insert(choice);
            let counts = per_branch
                .iter()
                .filter(|((candidate, _), _)| *candidate == choice)
                .map(|((_, branch), count)| (*branch, *count))
                .collect::<Vec<_>>();
            if counts.is_empty() {
                continue;
            }
            let agreed = if restricted_to_one_path {
                // One path survives, so there is nothing to agree with.
                counts.iter().try_fold(0_usize, |sum, (_, count)| {
                    Some(sum.saturating_add((*count)?))
                })?
            } else {
                let expected = arity_of_choice.get(&choice).copied()?;
                let first = counts.first().and_then(|(_, count)| *count)?;
                if counts.len() != expected || counts.iter().any(|(_, count)| *count != Some(first))
                {
                    return None;
                }
                first
            };
            let parent = counts.first().and_then(|(branch, _)| {
                ancestry.get(&(choice, *branch)).and_then(|chain| {
                    chain
                        .split_last()
                        .and_then(|(_, rest)| rest.last().copied())
                })
            });
            for (branch, _) in &counts {
                per_branch.remove(&(choice, *branch));
            }
            match parent {
                Some(parent_key) => {
                    let slot = per_branch.entry(parent_key).or_insert(Some(0));
                    *slot = slot.map(|sum| sum.saturating_add(agreed));
                }
                None => total = total.saturating_add(agreed),
            }
        }
        Some(total)
    }

    /// Greatest number of children `refs` can contribute on any single parse, or
    /// `None` when unbounded. Sequential refs add; branches of a choice are
    /// alternatives, so the widest one wins. Choices fold innermost-first so a
    /// nested choice's maximum lands in its enclosing branch rather than being
    /// summed alongside it.
    fn widest_child_count<'a>(refs: impl Iterator<Item = &'a ElementRef>) -> Option<usize> {
        let refs = refs.collect::<Vec<_>>();
        let mut total = Some(0_usize);
        let mut per_branch: BTreeMap<(usize, usize), Option<usize>> = BTreeMap::new();
        let mut depth_of_choice: BTreeMap<usize, usize> = BTreeMap::new();
        let mut ancestry: BTreeMap<(usize, usize), Vec<(usize, usize)>> = BTreeMap::new();
        for candidate in &refs {
            for (depth, &(choice, branch)) in candidate.choice_branch.iter().enumerate() {
                depth_of_choice
                    .entry(choice)
                    .and_modify(|existing| *existing = (*existing).min(depth))
                    .or_insert(depth);
                ancestry.insert((choice, branch), candidate.choice_branch[..=depth].to_vec());
            }
        }
        let add = |slot: &mut Option<usize>, value: Option<usize>| {
            *slot = match (*slot, value) {
                (Some(total), Some(next)) => Some(total.saturating_add(next)),
                _ => None,
            };
        };
        for candidate in &refs {
            match candidate.choice_branch.last() {
                None => add(&mut total, candidate.cardinality.max),
                Some(&key) => {
                    let slot = per_branch.entry(key).or_insert(Some(0));
                    add(slot, candidate.cardinality.max);
                }
            }
        }
        let mut processed: BTreeSet<usize> = BTreeSet::new();
        while let Some(choice) = per_branch
            .keys()
            .map(|(choice, _)| *choice)
            .filter(|choice| !processed.contains(choice))
            .max_by_key(|choice| depth_of_choice.get(choice).copied().unwrap_or(0))
        {
            processed.insert(choice);
            let counts = per_branch
                .iter()
                .filter(|((candidate, _), _)| *candidate == choice)
                .map(|((_, branch), count)| (*branch, *count))
                .collect::<Vec<_>>();
            if counts.is_empty() {
                continue;
            }
            let widest = counts
                .iter()
                .try_fold(0_usize, |widest, (_, count)| Some(widest.max((*count)?)));
            let parent = counts.first().and_then(|(branch, _)| {
                ancestry.get(&(choice, *branch)).and_then(|chain| {
                    chain
                        .split_last()
                        .and_then(|(_, rest)| rest.last().copied())
                })
            });
            for (branch, _) in &counts {
                per_branch.remove(&(choice, *branch));
            }
            match parent {
                Some(parent_key) => {
                    let slot = per_branch.entry(parent_key).or_insert(Some(0));
                    add(slot, widest);
                }
                None => add(&mut total, widest),
            }
        }
        total
    }

    /// Whether the action sits inside the branch that separates `element` from
    /// `candidate` — i.e. inside the label's own branch of the choice that makes the
    /// two mutually exclusive. Only then can the candidate be dismissed: the action
    /// cannot run on the branch that would supply it.
    ///
    /// Judged per choice rather than rule-wide, because an action confined to some
    /// *unrelated* later choice says nothing about an earlier one.
    fn action_inside_separating_branch(
        element: &ElementRef,
        candidate: &ElementRef,
        action_branches: Option<&[(usize, usize)]>,
    ) -> bool {
        let Some(branches) = action_branches else {
            // An unscoped body runs whatever branch matched.
            return false;
        };
        element.choice_branch.iter().any(|&(choice, branch)| {
            // A choice that separates them...
            candidate
                .choice_branch
                .iter()
                .any(|&(other, other_branch)| other == choice && other_branch != branch)
                // ...and whose label-side branch encloses the action.
                && branches.contains(&(choice, branch))
        })
    }

    /// Whether two per-alternative resolutions lower to the same read, so one
    /// translation can stand for both. The fields compared are exactly those
    /// `translate_element_read` consumes to pick a read: list mode, block mode,
    /// and the target it queries. Two block labels are equivalent regardless of
    /// their token sets, because the block read ignores them.
    fn same_label_read(left: &(ElementRef, usize), right: &(ElementRef, usize)) -> bool {
        if left.0.is_list != right.0.is_list {
            return false;
        }
        // Token-backed resolutions can be equivalent across source forms (`x=A` is
        // token-mode, `x='a'` is block-mode) because both lower to the same
        // `child_tokens(A)` query — but their occurrences are counted in different
        // units: a block read indexes *every* terminal child, a token read only
        // same-type children. `x='a' | A x=A` has both reporting 1 while meaning
        // different children, and `A x='a' B | A x=A C` has them meaning the same
        // child while reporting 1 and 1 only by coincidence.
        //
        // Rather than guess, mixed-mode pairs merge only when *neither* has anything
        // ahead of it — occurrence zero in both systems *and* no preceding terminal
        // of any type. Occurrence zero alone is not enough: in `B x=A | x='a'` the
        // symbolic side reports same-token occurrence 0 while sitting at terminal
        // position 1, and merging it with the literal's terminal 0 exposed `B`.
        // Same-mode pairs compare directly.
        if !left.0.token_types.is_empty() && left.0.token_types == right.0.token_types {
            if left.0.is_block == right.0.is_block {
                return left.1 == right.1;
            }
            // The mixed-mode merge works because both sides lower to the same
            // *scalar* `nth(0)`. A list read has no such common form: token mode
            // yields an iterator (`child_tokens(A)`), block mode a `String` — it
            // resolves `target` against `ctx.token_types`, and a literal target
            // (`xs+='a'`) is not a key there, so it falls through to the positional
            // block read. Merging the two emitted `.collect()` on a `String`.
            if left.0.is_list {
                return false;
            }
            return left.1 == 0
                && right.1 == 0
                && left.0.leading_terminal
                && right.0.leading_terminal;
        }
        if left.0.is_block != right.0.is_block {
            return false;
        }
        // Block reads are positional now, so two block labels agree only when
        // their terminal indices do: `x=(A | B) | C x=(A | B)` puts `x` at 0 and 1.
        if left.0.is_block && right.0.is_block {
            return left.1 == right.1;
        }
        left.1 == right.1 && left.0.target == right.0.target
    }

    /// `action_offset` is the byte offset of a *mid-rule* action body, or `None`
    /// for an `@after` / `@init` body that runs after the whole rule.
    fn resolve_label_in_alt(
        alt: &AltModel,
        label: &str,
        action_offset: Option<usize>,
    ) -> Option<(ElementRef, usize)> {
        let declarations = alt
            .refs
            .iter()
            .filter(|element| element.label.as_deref() == Some(label))
            .collect::<Vec<_>>();
        let element = *declarations.first()?;
        // A renamed sibling declaration (see the isolation below) still counts as
        // declaring the label: it binds it too, so it can never impersonate it.
        let declares_label = |candidate: &ElementRef| {
            candidate.label.as_deref().is_some_and(|name| {
                name == label || name.strip_suffix(SIBLING_DECLARATION_SUFFIX) == Some(label)
            })
        };
        // The generated read queries by rule index or *token type*, so a
        // differently-spelled terminal with the same type is the same child as
        // far as the read is concerned (`A : 'a';` makes `A` and `'a'` aliases).
        let same_target = |candidate: &ElementRef| {
            if candidate.cardinality.max == Some(0) {
                return false;
            }
            // A token *group* has no target yet still contributes a child of the
            // label's type when their sets overlap (`(xs+=A)? (A | B)`), so match
            // on token types whenever both sides have them — empty target or not.
            if element.token_types.is_empty() || candidate.token_types.is_empty() {
                return !candidate.target.is_empty() && candidate.target == element.target;
            }
            candidate
                .token_types
                .iter()
                .any(|token_type| element.token_types.contains(token_type))
        };
        // Whether `candidate` has been matched by the time the action body runs.
        // A mid-rule action executes at its source position, so a ref that starts
        // after it is still in the future and cannot affect the read; a ref in a
        // branch the action does not sit inside cannot have run either. An
        // `@after` body runs after everything, so every ref counts.
        // The choice ancestry the action itself sits in, derived from spans: the
        // action belongs to the innermost branch whose refs bracket its offset.
        // Refs from any *other* branch of those choices cannot have run.
        // The choice branches that syntactically *enclose* the action. A branch
        // encloses it only when the branch has a ref before the action AND no
        // sibling branch of the same choice has a ref after it — a sibling ref
        // afterwards means the choice is still open, i.e. the action follows the
        // whole group rather than sitting inside one branch. Nearest-preceding-ref
        // alone gets `(A | xs+=A) {…}` wrong, marking the action as confined to the
        // final branch when it actually runs for either.
        // The choice branches that syntactically enclose the action: those whose
        // *block* span contains the action's offset. Ref spans alone cannot decide
        // this — `(A | xs+=A) {…}` and `(A x=A {…} | B)` both put branch refs on
        // either side of the action — but the block's own extent can: in the first
        // the action sits after the closing paren, in the second inside it.
        let action_branches = action_offset.map(|offset| {
            // For each enclosing choice, the *one* branch the action sits in: the
            // branch whose own refs span the offset. Refs of an earlier sibling
            // also precede the action and share the choice's block span, so
            // collecting every preceding ref's tags would record mutually
            // conflicting branches (`(B | C x=A? A {…})` would claim both).
            //
            // A branch contains the action when some ref of it starts before the
            // offset and no ref of a *later* sibling does — source order means a
            // later branch having started implies the action is past this one.
            let mut chosen: Vec<(usize, usize)> = Vec::new();
            // Every (choice, branch) whose *branch text* contains the action. This
            // reads the branch's own span rather than inferring from ref positions,
            // so a branch holding only an action or predicate — which emits no
            // `ElementRef` — is still identified (`x=A? (A | {$x.text})`).
            for candidate in &alt.refs {
                for ((&key, &(branch_start, branch_end)), &(choice_start, choice_end)) in candidate
                    .choice_branch
                    .iter()
                    .zip(&candidate.branch_spans)
                    .zip(&candidate.choice_spans)
                {
                    let inside_choice = choice_start <= offset && offset < choice_end;
                    let inside_branch = branch_start <= offset && offset < branch_end;
                    if inside_choice && inside_branch && !chosen.contains(&key) {
                        chosen.push(key);
                    }
                }
            }
            // A choice enclosing the action but with no branch claiming it means the
            // action sits in a ref-free branch: nothing of that branch has matched,
            // so record it as its own branch so siblings are excluded.
            for candidate in &alt.refs {
                for (&(choice, _), &(choice_start, choice_end)) in
                    candidate.choice_branch.iter().zip(&candidate.choice_spans)
                {
                    if choice_start <= offset
                        && offset < choice_end
                        && !chosen
                            .iter()
                            .any(|&(chosen_choice, _)| chosen_choice == choice)
                    {
                        // usize::MAX marks "a branch with no refs of its own".
                        chosen.push((choice, usize::MAX));
                    }
                }
            }
            chosen
        });
        let branch_confined = action_branches
            .as_ref()
            .is_some_and(|branches| !branches.is_empty());
        // A ref inside a group that also encloses the action has run: the action
        // only executes when that group was taken. Its cardinality still reports
        // `min: 0` from the group's `?`, so use the quantifier-free figure —
        // `(A x=A {…})?` has exactly one `A` before the label whenever the action
        // runs at all.
        //
        // *Every* group the ref sits in must enclose the action, not merely one: an
        // inner group that closed before the action proves nothing, so
        // `((q)? x=q {…})?` must not treat the inner `(q)?` as matched.
        // A ref is exactly-once on the action's path when every group that *relaxed*
        // its lower bound is one the action also sits inside — the action running
        // proves those groups were taken. Groups that impose nothing (a mandatory
        // `(…)`) are irrelevant whether or not they enclose the action, so requiring
        // all of them to would reject `((q) x=q {…})?`, where the inner group is
        // mandatory and already closed.
        let on_taken_group = |candidate: &ElementRef| {
            action_offset.is_some_and(|offset| {
                let encloses = |group: &GroupSpan| group.start <= offset && offset < group.end;
                // A *repeated* group that has closed still contributes an unknown
                // number of children, so knowing it ran does not fix the count:
                // `((A B)+ x=A {…})?` has a variable run of `A` before the label.
                if candidate
                    .group_spans
                    .iter()
                    .any(|group| group.repeated && !encloses(group))
                {
                    return false;
                }
                let relaxing = candidate
                    .group_spans
                    .iter()
                    .filter(|group| group.optional)
                    .collect::<Vec<_>>();
                !relaxing.is_empty() && relaxing.iter().all(|group| encloses(group))
            })
        };
        // Whether a ref can have run before the action, given that ancestry. An
        // action after the whole choice (`(x=A | A) {$x}`) has no branch tag, so
        // every branch counts; one written inside a branch excludes its siblings —
        // including when the label itself is in another branch (`(e | xs+=e {…})`).
        let on_action_path = |candidate: &ElementRef| {
            action_branches.as_ref().is_none_or(|branches| {
                !candidate.choice_branch.iter().any(|(choice, branch)| {
                    branches
                        .iter()
                        .any(|(a_choice, a_branch)| choice == a_choice && branch != a_branch)
                })
            })
        };
        let matched_at_action = |candidate: &ElementRef| {
            action_offset.is_none_or(|offset| {
                let started = candidate.span.is_none_or(|(start, _)| start < offset);
                started && on_action_path(candidate)
            })
        };
        // Two declarations can share one read when the generated query is the
        // same. For token-backed refs that is the token type, not the source form:
        // `x=A` and `x='a'` differ in spelling and block-ness yet query alike.
        let same_read_as_element = |candidate: &ElementRef| {
            candidate.is_list == element.is_list
                && if candidate.token_types.is_empty() || element.token_types.is_empty() {
                    candidate.target == element.target && candidate.is_block == element.is_block
                } else {
                    candidate.token_types == element.token_types
                }
        };

        if element.is_list {
            // `translate_element_read` lowers a list label to a per-target child
            // iterator, which needs a rule or token *target*. A list over a token
            // group (`xs+=(A | B)`) has none, so the read would fall through to
            // the scalar block path and emit `.last()…collect()` — code that does
            // not compile. Leave it unresolved instead.
            if element.target.is_empty() {
                return None;
            }
            // A list read yields every child of *one* query, so repeated declarations
            // are the normal idiom (`xs+=e (op xs+=e)+`) only while they all name that
            // same query. `xs+=A xs+=B` would iterate `A` alone and drop every `B`.
            // The query is the token type for token-backed refs, not the spelling:
            // `xs+=A B | xs+='a' C` binds one type through two source forms.
            let same_query = |candidate: &ElementRef| {
                if element.token_types.is_empty() || candidate.token_types.is_empty() {
                    candidate.target == element.target
                } else {
                    candidate.token_types == element.token_types
                }
            };
            if declarations
                .iter()
                .any(|candidate| !same_query(candidate) || !candidate.is_list)
            {
                return None;
            }
            // What the read cannot express is exclusion, so the label resolves
            // only when no *already-matched* same-target element sits outside it.
            // A trailing `A` in `r : xs+=A {$xs} A;` has not been matched when the
            // action runs, so it cannot pollute the iterator.
            let exclusive = alt.refs.iter().all(|candidate| {
                declares_label(candidate)
                    || !same_target(candidate)
                    || !matched_at_action(candidate)
            });
            return exclusive.then(|| (element.clone(), 0));
        }
        // Only declarations the action can actually observe constrain its read. Two
        // rule it out: one in a sibling branch, which never runs alongside a
        // branch-confined action (`(x=A {$x} | x=A+ B)`), and one *after* the action,
        // which has not assigned the label yet (`x=A {$x} x=A` reads the first
        // assignment unambiguously).
        let relevant = declarations
            .iter()
            .copied()
            .filter(|candidate| {
                (!branch_confined || on_action_path(candidate)) && matched_at_action(candidate)
            })
            .collect::<Vec<_>>();
        let declarations = if relevant.is_empty() {
            declarations
        } else {
            relevant
        };
        let element = *declarations.first()?;
        // A single label read is one positional lookup. Several declarations can
        // still share it when each lowers to the same query — mutually exclusive
        // branches holding `x=A` at the same occurrence do. What cannot be served
        // is declarations that query differently (`(x=A | x=B)`), or that could
        // both be present and so want different positions.
        if declarations.iter().any(|candidate| {
            !same_read_as_element(candidate)
                || (!std::ptr::eq(*candidate, element) && candidate.can_coexist_with(element))
        }) {
            return None;
        }
        // Deriving the read from the *first* declaration and probing the others
        // property-by-property kept missing a dimension — occurrence and repetition
        // among them. Instead resolve each declaration on its own and require the
        // results to agree, so one read demonstrably serves every branch:
        // `(A x=A B | x=A C)` wants occurrence 1 then 0, and `(x=A B | x=A+ C)`
        // wants a first-match read then a last-match one.
        if declarations.len() > 1 {
            let mut resolutions = Vec::with_capacity(declarations.len());
            for candidate in &declarations {
                let position = alt
                    .refs
                    .iter()
                    .position(|other| std::ptr::eq(other, *candidate))?;
                let mut alone = alt.clone();
                // Isolate this declaration by *renaming* the others rather than
                // clearing their labels. Clearing would reclassify a fellow
                // declaration as an unlabeled impostor and trip the shadow check —
                // `(x=A | x=A)` would reject itself. Renaming keeps them labeled, so
                // `declares_label` still exempts them, while only one answers to
                // `label`.
                let shadow_name = format!("{label}{SIBLING_DECLARATION_SUFFIX}");
                for (index, ref_at) in alone.refs.iter_mut().enumerate() {
                    if index != position && ref_at.label.as_deref() == Some(label) {
                        ref_at.label = Some(shadow_name.clone());
                    }
                }
                resolutions.push(Self::resolve_label_in_alt(&alone, label, action_offset)?);
            }
            let first = resolutions.first()?.clone();
            if resolutions
                .iter()
                .any(|resolution| !Self::same_label_read(resolution, &first))
            {
                return None;
            }
            return Some(first);
        }
        // `element` is borrowed from `alt.refs`, so identity holds — but compare by
        // value as a fallback, because the sibling-isolation rename above clones the
        // alternative and a filtered `declarations` list can outlive that borrow.
        let position = alt
            .refs
            .iter()
            .position(|candidate| std::ptr::eq(candidate, element))
            .or_else(|| alt.refs.iter().position(|candidate| candidate == element))?;
        let (before, after) = (&alt.refs[..position], &alt.refs[position + 1..]);
        // `translate_element_read` routes on `is_block`, which covers labeled
        // groups and *literal* terminals alike (`x='b'`), so the occurrence has to
        // be computed the same way for both — keying on an empty target here would
        // leave a literal label counting same-target children while its read walks
        // every terminal.
        if element.is_block {
            // A *repeated* block label (`x=(A | B)+`) is overwritten each iteration,
            // so ANTLR exposes the last match while a positional read pins the first.
            // The non-block path already declines this; do the same rather than read
            // the wrong iteration.
            if element.cardinality.is_repeated() {
                return None;
            }
            // A block label has no single target to query, so its read walks the
            // context's terminal children by position. The index is the number of
            // terminals matched ahead of the block on this parse path — every
            // terminal counts, not just ones sharing the block's token set, since
            // each is a distinct child of the same context.
            // `on_action_path` already narrowed these to one parse path (when the
            // action is inside a branch), so branches that survive simply count.
            // Confinement to one *outer* branch does not restrict a nested choice
            // inside it: `((a=A | b=B) x=(C | D) {…} | E)` still has the `A`/`B`
            // branches to reconcile, and calling the count path-restricted summed
            // them. Strip the tags of choices the action is genuinely inside, then
            // let any surviving tag force cross-branch agreement.
            let counted = before
                .iter()
                .filter(|candidate| {
                    !candidate.token_types.is_empty()
                        && on_action_path(candidate)
                        // Only children already matched when the action runs affect
                        // its read. For a *forward* label (`A {$x.text} B? x=(C|D)`)
                        // the prefix is entirely in the future, so counting `B?`
                        // made the index inexact and fell back to `last()` — which
                        // returns the already-matched `A`.
                        && matched_at_action(candidate)
                        // A ref in a branch this label cannot reach never precedes it:
                        // in `(x=A | x='a')` the sibling declaration is not a prefix
                        // terminal of the literal's path.
                        && candidate.can_coexist_with(element)
                })
                .cloned()
                .map(|mut candidate| {
                    if let Some(branches) = action_branches.as_ref() {
                        candidate.retain_choices(|choice| {
                            !branches.iter().any(|&(taken, _)| taken == choice)
                        });
                    }
                    candidate
                })
                .collect::<Vec<_>>();
            let restricted = counted
                .iter()
                .all(|candidate| candidate.choice_branch.is_empty());
            let terminals_before = Self::exact_child_count(counted.iter(), restricted);
            // Without a fixed index the read falls back to the most recent
            // terminal, which is only right when nothing has been matched since.
            // A sibling branch that puts a terminal at the same index supplies the
            // child this read selects on a parse where the label is unset:
            // `((x=(A | B)) | C) {$x}` reads `C` on the `C` branch.
            if let Some(index) = terminals_before {
                // A sibling branch's terminal can only be mistaken for the label
                // when the read actually runs on that branch. An action confined to
                // the label's own branch never executes there, so the sibling is
                // irrelevant — `(x=(A | B) {…} | C)` is safe even though `C` sits at
                // the same index.
                let sibling_at_index = !branch_confined
                    && alt.refs.iter().any(|candidate| {
                        // Another declaration of the same label binds it too, so it
                        // can never impersonate it — `(x=A | x='a')` is one label
                        // over two branches, not a label and an impostor.
                        !declares_label(candidate)
                            && !candidate.can_coexist_with(element)
                            && !candidate.token_types.is_empty()
                            && candidate.cardinality.max != Some(0)
                            && Self::can_occupy_terminal_index(alt, candidate, index)
                    });
                if sibling_at_index {
                    return None;
                }
                // ROOT J: a fixed index is not enough when the label is *optional* —
                // `x=(A | B)? C {…}` puts `C` at index 0 whenever the block is
                // absent, so the read would report it as the label's token.
                let optional_here = if on_taken_group(element) {
                    element.group_local_cardinality.min == 0
                } else {
                    element.cardinality.min == 0
                };
                if optional_here
                    && after.iter().any(|candidate| {
                        !candidate.token_types.is_empty()
                            && candidate.cardinality.max != Some(0)
                            && matched_at_action(candidate)
                    })
                {
                    return None;
                }
            }
            return terminals_before.map_or_else(
                || {
                    let displaced = after.iter().any(|candidate| {
                        !candidate.token_types.is_empty()
                            && candidate.cardinality.max != Some(0)
                            && matched_at_action(candidate)
                    });
                    (!displaced).then(|| (element.clone(), usize::MAX))
                },
                |index| Some((element.clone(), index)),
            );
        }

        // A repeated single label (`(x=A)+`) is overwritten on every iteration,
        // so ANTLR exposes the *latest* match. The read here is a fixed
        // `nth(i)`, which would pin the first one; only the accessor path can
        // express `.last()`. Leave it unresolved rather than read the wrong
        // iteration.
        if element.cardinality.is_repeated() {
            return None;
        }
        // Single label: count the children ahead of it that the read would also
        // select, bailing as soon as one contributes an unfixed number. A token
        // *group* has no target yet still produces a child of the label's token
        // type when their sets overlap (`(A | B) x=A`), so it must be counted —
        // and since only some of its members match, its contribution is not
        // exact and the label declines.
        let counts_toward_occurrence = |candidate: &ElementRef| {
            if element.token_types.is_empty() || candidate.token_types.is_empty() {
                return candidate.target == element.target;
            }
            candidate
                .token_types
                .iter()
                .any(|token_type| element.token_types.contains(token_type))
        };
        // A token *group* only sometimes yields a matching child, so its count is
        // exact only when every member is one the read selects.
        if before.iter().any(|candidate| {
            counts_toward_occurrence(candidate)
                && !candidate.token_types.is_empty()
                && !candidate
                    .token_types
                    .iter()
                    .all(|token_type| element.token_types.contains(token_type))
        }) {
            return None;
        }
        // Count only children on the label's own parse path, and — when the action
        // is confined to a branch — count the surviving branch as that path rather
        // than demanding agreement from branches already filtered out.
        let counted = before
            .iter()
            .filter(|candidate| {
                // Only children already matched when the action runs can affect its
                // read. An inline action *before* the label sees none of them, so a
                // later unbounded run must not poison the count:
                // `r : {$x.text} A* x=A EOF;` reads an empty list, whatever follows.
                counts_toward_occurrence(candidate)
                    && matched_at_action(candidate)
                    && candidate.can_coexist_with(element)
            })
            .cloned()
            .map(|mut candidate| {
                if on_taken_group(&candidate) {
                    // The enclosing group is taken on the action's path, so the
                    // group's own quantifier no longer relaxes this ref.
                    candidate.cardinality = candidate.group_local_cardinality;
                    candidate.branch_local_cardinality = candidate.group_local_cardinality;
                }
                // Choices the *label* is inside are settled on its path, so those
                // tags carry no remaining alternation. Tags that survive belong to
                // choices the label sits outside of, whose branches are genuinely
                // alternative.
                candidate.retain_choices(|choice| {
                    !element
                        .choice_branch
                        .iter()
                        .any(|&(taken, _)| taken == choice)
                });
                candidate
            })
            .collect::<Vec<_>>();
        // `can_coexist_with` keeps every branch of a choice the label is outside of,
        // so it does not by itself select one path: `(A B | A C) x=A` would sum both
        // prefixes. Only claim path-restriction once no alternation remains.
        let restricted = counted
            .iter()
            .all(|candidate| candidate.choice_branch.is_empty());
        let occurrence = Self::exact_child_count(counted.iter(), restricted)?;
        // An optional label is displaced by a following same-target child that can
        // slide into its position, whether that child is mandatory (`x=A? A`) or
        // optional (`(pred x=A)? A?` — the follower may consume the only token).
        // Two kinds cannot: a ref from a sibling branch of the same choice, since
        // no parse contains both, and — for a mid-rule action — a ref that starts
        // after the action and so has not been matched when the read runs.
        // `matched_at_action` already folds in coexistence when the action is
        // confined to the label's branch; applying it again here would also
        // exclude siblings for an action that runs after the whole choice.
        // Another *declaration* of the same label is not a shadow — it binds the
        // label too, and the guard above already proved they share one read.
        // The label's own `min: 0` may come from a group the action shares, in which
        // case it is *not* optional relative to the action: `(A x=A {…})?` only runs
        // the action when the group matched, so `x` is bound.
        let element_optional_here = if on_taken_group(element) {
            element.group_local_cardinality.min == 0
        } else {
            element.cardinality.min == 0
        };
        // A sibling branch's child cannot slide into the label's slot *within a
        // parse that bound the label* — the two never coexist. It is still a hazard
        // when the read may run with the label unset, which is when the action is
        // not inside the branch that separates them. That has to be judged per
        // *choice*: a rule-wide flag would let an action confined to some unrelated
        // later choice exempt an earlier sibling
        // (`({false}? x=A | A) (B {$x} | C)`).
        let excluded_by_confinement = |candidate: &ElementRef| {
            !candidate.can_coexist_with(element)
                && Self::action_inside_separating_branch(
                    element,
                    candidate,
                    action_branches.as_deref(),
                )
        };
        let shadowed_when_absent = element_optional_here
            && (after.iter().any(|candidate| {
                !declares_label(candidate)
                    && same_target(candidate)
                    && matched_at_action(candidate)
                    && !excluded_by_confinement(candidate)
            })
            // A same-target sibling *before* the label impersonates it just as well:
            // in `(A | x=A) {$x}` the unlabeled `A` is the only child of its type on
            // its own branch, so `child_tokens(A).nth(0)` reports it. Only
            // non-coexisting refs matter here — a ref the label coexists with is a
            // genuine prefix and is already folded into `occurrence`.
            || before.iter().any(|candidate| {
                !declares_label(candidate)
                    && same_target(candidate)
                    && matched_at_action(candidate)
                    && !candidate.can_coexist_with(element)
                    && !excluded_by_confinement(candidate)
            }));
        (!shadowed_when_absent).then_some((element.clone(), occurrence))
    }
}

/// Generated attrs struct name for a rule.
pub(crate) fn attrs_struct_name(rule_index: usize) -> String {
    format!("__RuleAttrs{rule_index}")
}

/// Translates every `$…` reference in an embedded body to Rust.
pub(crate) fn translate_body(body: &str, ctx: &TranslationCtx<'_>) -> io::Result<String> {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    let macro_rules_ranges = macro_rules_definition_ranges(body);
    let mut body_offset = 0;
    while let Some(dollar) = find_dollar(rest) {
        let absolute_dollar = body_offset + dollar;
        out.push_str(&rest[..dollar]);
        if macro_rules_ranges
            .iter()
            .any(|range| range.contains(&absolute_dollar))
        {
            out.push('$');
            rest = &rest[dollar + 1..];
            body_offset = absolute_dollar + 1;
            continue;
        }
        let after = &rest[dollar + 1..];
        let name_len = after
            .find(|ch: char| ch != '_' && !ch.is_ascii_alphanumeric())
            .unwrap_or(after.len());
        if name_len == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("stray $ in embedded action: {body}"),
            ));
        }
        let name = &after[..name_len];
        // Optional `.suffix`.
        let mut consumed = name_len;
        let mut suffix: Option<&str> = None;
        if after[name_len..].starts_with('.') {
            let suffix_text = &after[name_len + 1..];
            let suffix_len = suffix_text
                .find(|ch: char| ch != '_' && !ch.is_ascii_alphanumeric())
                .unwrap_or(suffix_text.len());
            if suffix_len > 0 {
                // Only treat it as an attribute suffix when it is not a
                // method call — `$ctx.to_string_tree(...)` keeps its call.
                let after_suffix = suffix_text[suffix_len..].trim_start();
                let is_call = after_suffix.starts_with('(');
                if !is_call {
                    suffix = Some(&suffix_text[..suffix_len]);
                    consumed = name_len + 1 + suffix_len;
                } else if name == "ctx"
                    && (suffix_text[..suffix_len].ends_with("_children")
                        || suffix_text[..suffix_len].ends_with("_all"))
                    && after_suffix.starts_with("()")
                {
                    // `$ctx.<rule>_children()` (or the legacy `_all()` form) is
                    // an active-context collection read. Consume the empty
                    // parens along with the suffix.
                    suffix = Some(&suffix_text[..suffix_len]);
                    let call_end = suffix_text[suffix_len..]
                        .find(')')
                        .map_or(suffix_len, |close| suffix_len + close + 1);
                    consumed = name_len + 1 + call_end;
                }
            }
        }
        let translated = translate_reference(name, suffix, ctx, body)?;
        out.push_str(&translated);
        rest = &rest[dollar + 1 + consumed..];
        body_offset = absolute_dollar + 1 + consumed;
    }
    out.push_str(rest);
    Ok(out)
}

/// Translates ANTLR attributes and the observed antlr4rust parser ABI.
///
/// Predicates are always block expressions so a rendered body may contain
/// statements before its final boolean expression. Each `_localctx` read
/// materializes a fresh typed-context view so earlier writes in the same body
/// are visible through its copied compatibility attributes.
#[cfg(test)]
pub(crate) fn translate_parser_body(
    body: &str,
    ctx: &TranslationCtx<'_>,
    active_context_type: &str,
    antlr4rust_token_aliases: &BTreeSet<String>,
    kind: ParserBodyKind,
) -> io::Result<ParserBodyTranslation> {
    translate_parser_body_with_alias_module(
        body,
        ctx,
        active_context_type,
        antlr4rust_token_aliases,
        Antlr4RustNames {
            token_alias_module: ANTLR4RUST_TOKEN_ALIAS_MODULE,
            context_wrapper: ANTLR4RUST_CONTEXT_WRAPPER,
            input_facade: ANTLR4RUST_INPUT_FACADE,
        },
        kind,
    )
}

pub(crate) fn translate_parser_body_with_alias_module(
    body: &str,
    ctx: &TranslationCtx<'_>,
    active_context_type: &str,
    antlr4rust_token_aliases: &BTreeSet<String>,
    names: Antlr4RustNames<'_>,
    kind: ParserBodyKind,
) -> io::Result<ParserBodyTranslation> {
    let Antlr4RustNames {
        token_alias_module,
        context_wrapper,
        input_facade,
    } = names;
    let translated = translate_body(body, ctx)?;
    let live_attrs = if ctx.model.rules[ctx.rule_index].has_attrs() {
        "&__attrs"
    } else {
        "&()"
    };
    let local_context = format!(
        "__active_context_view_with_attrs::<{active_context_type}<'_, __ActiveParserContext>>(\n    &__ctx,\n    {live_attrs},\n    self.base.active_invocation_states(),\n    self.base.parse_tree_storage(),\n    self.base.token_store(),\n)"
    );
    let local_context = format!("{local_context}.map({context_wrapper})");
    let LoweredAntlr4RustBody {
        source,
        uses_input,
        uses_local_context,
        token_aliases,
        ..
    } = lower_antlr4rust_surface(
        &translated,
        antlr4rust_token_aliases,
        token_alias_module,
        input_facade,
        Some(&local_context),
        Antlr4RustSourceKind::Body,
    )?;
    if kind == ParserBodyKind::Action && !uses_local_context {
        return Ok(ParserBodyTranslation {
            source,
            uses_input,
            uses_local_context,
            token_aliases,
        });
    }

    let mut out = String::from("{\n");
    out.push_str(&source);
    out.push_str("\n}");
    Ok(ParserBodyTranslation {
        source: out,
        uses_input,
        uses_local_context,
        token_aliases,
    })
}

pub(crate) fn action_references(body: &str) -> Vec<ActionReference<'_>> {
    let macro_rules_ranges = macro_rules_definition_ranges(body);
    generic_action_references(body)
        .into_iter()
        .filter(|reference| {
            !macro_rules_ranges
                .iter()
                .any(|range| range.contains(&reference.name_offset))
        })
        .collect()
}

/// Finds the next `$` that is outside a string literal.
pub(crate) fn find_dollar(text: &str) -> Option<usize> {
    let mut quoted = false;
    let mut escaped = false;
    for (index, ch) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '$' if !quoted => return Some(index),
            _ => {}
        }
    }
    None
}

pub(crate) fn lex_rust_body(body: &str) -> Vec<RustLexeme> {
    let bytes = body.as_bytes();
    let mut lexemes = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let start = offset;
        let kind = if bytes[offset].is_ascii_whitespace() {
            offset = skip_while(bytes, offset, is_ascii_whitespace);
            RustLexemeKind::Trivia
        } else if body[offset..].starts_with("//") {
            offset = body[offset..]
                .find('\n')
                .map_or(body.len(), |newline| offset + newline);
            RustLexemeKind::Trivia
        } else if body[offset..].starts_with("/*") {
            offset = block_comment_end(body, offset);
            RustLexemeKind::Trivia
        } else if let Some(end) = raw_literal_end(body, offset) {
            offset = end;
            RustLexemeKind::Literal
        } else if let Some(end) = quoted_literal_end(body, offset) {
            offset = end;
            RustLexemeKind::Literal
        } else if let Some(end) = raw_identifier_end(body, offset) {
            offset = end;
            RustLexemeKind::Identifier
        } else if let Some(end) = rust_identifier_end(body, offset) {
            offset = end;
            RustLexemeKind::Identifier
        } else if bytes[offset].is_ascii_punctuation() {
            offset += 1;
            RustLexemeKind::Punctuation(bytes[offset - 1])
        } else {
            offset += body[offset..]
                .chars()
                .next()
                .expect("offset is within the string")
                .len_utf8();
            RustLexemeKind::Other
        };
        lexemes.push(RustLexeme {
            kind,
            start,
            end: offset,
        });
    }
    lexemes
}

pub(crate) fn skip_while(bytes: &[u8], mut offset: usize, predicate: fn(u8) -> bool) -> usize {
    while bytes.get(offset).copied().is_some_and(predicate) {
        offset += 1;
    }
    offset
}

pub(crate) const fn is_ascii_whitespace(byte: u8) -> bool {
    byte.is_ascii_whitespace()
}

pub(crate) fn block_comment_end(body: &str, start: usize) -> usize {
    let bytes = body.as_bytes();
    let mut depth = 1;
    let mut offset = start + 2;
    while offset + 1 < bytes.len() {
        match &bytes[offset..offset + 2] {
            b"/*" => {
                depth += 1;
                offset += 2;
            }
            b"*/" => {
                depth -= 1;
                offset += 2;
                if depth == 0 {
                    return offset;
                }
            }
            _ => offset += 1,
        }
    }
    body.len()
}

pub(crate) fn raw_literal_end(body: &str, start: usize) -> Option<usize> {
    let rest = &body[start..];
    let prefix = ["br", "cr", "r"]
        .into_iter()
        .find(|prefix| rest.starts_with(prefix))?;
    let mut quote = start + prefix.len();
    while body.as_bytes().get(quote) == Some(&b'#') {
        quote += 1;
    }
    if body.as_bytes().get(quote) != Some(&b'"') {
        return None;
    }
    let hashes = quote - start - prefix.len();
    let closing = format!("\"{}", "#".repeat(hashes));
    let content = quote + 1;
    Some(
        body[content..]
            .find(&closing)
            .map_or(body.len(), |end| content + end + closing.len()),
    )
}

pub(crate) fn raw_identifier_end(body: &str, start: usize) -> Option<usize> {
    if body.as_bytes().get(start..start + 2) != Some(b"r#") {
        return None;
    }
    rust_identifier_end(body, start + 2)
}

pub(crate) fn quoted_literal_end(body: &str, start: usize) -> Option<usize> {
    let bytes = body.as_bytes();
    let (quote, content) = match bytes.get(start..start + 2) {
        Some([b'b' | b'c', b'"']) => (b'"', start + 2),
        Some([b'b', b'\'']) => (b'\'', start + 2),
        _ if bytes[start] == b'"' => (b'"', start + 1),
        _ if bytes[start] == b'\'' => (b'\'', start + 1),
        _ => return None,
    };
    if quote == b'\'' {
        return char_literal_end(body, content);
    }

    let mut offset = content;
    let mut escaped = false;
    while offset < bytes.len() {
        if escaped {
            escaped = false;
        } else if bytes[offset] == b'\\' {
            escaped = true;
        } else if bytes[offset] == quote {
            return Some(offset + 1);
        }
        offset += 1;
    }
    Some(body.len())
}

pub(crate) fn char_literal_end(body: &str, content: usize) -> Option<usize> {
    let bytes = body.as_bytes();
    let end = if bytes.get(content) == Some(&b'\\') {
        match bytes.get(content + 1).copied()? {
            b'x' => content.checked_add(4)?,
            b'u' if bytes.get(content + 2) == Some(&b'{') => {
                content + 3 + body[content + 3..].find('}')? + 1
            }
            _ => content.checked_add(2)?,
        }
    } else {
        content
            + body[content..]
                .chars()
                .next()
                .filter(|ch| *ch != '\'' && *ch != '\n' && *ch != '\r')?
                .len_utf8()
    };
    (bytes.get(end) == Some(&b'\'')).then_some(end + 1)
}

pub(crate) fn translate_reference(
    name: &str,
    suffix: Option<&str>,
    ctx: &TranslationCtx<'_>,
    body: &str,
) -> io::Result<String> {
    // Special names first.
    match (name, suffix) {
        ("ctx", None) => return Ok("(&__ctx)".to_owned()),
        ("ctx", Some(member)) => return translate_ctx_member(member, ctx, body),
        ("text", None) => return Ok(text_expression(ctx)),
        ("_p", None) => return Ok("__precedence".to_owned()),
        ("parser", None) => return Ok("self".to_owned()),
        ("start", None) => {
            return Ok("__ctx.start(self.base.token_store())".to_owned());
        }
        _ => {}
    }
    let rule = ctx.rule();
    // Labels shadow attrs; attrs shadow rule/token names.
    if let Some((element, occurrence)) = ctx.resolve_label(name) {
        return translate_element_read(&element, occurrence, suffix, ctx, body);
    }
    if rule.attr(name).is_some() {
        let mut expr = format!("__attrs.{}", escape_keyword(name));
        if let Some(suffix) = suffix {
            let _ = write!(expr, ".{suffix}");
        }
        return Ok(expr);
    }
    if let Some(target_rule) = ctx.rule_index_by_name(name) {
        let element = ElementRef {
            label: None,
            target: name.to_owned(),
            token_types: Vec::new(),
            is_block: false,
            is_list: false,
            cardinality: ChildCardinality::ONE,
            stable_accessor: false,
            choice_branch: Vec::new(),
            choice_arity: Vec::new(),
            choice_spans: Vec::new(),
            group_spans: Vec::new(),
            branch_spans: Vec::new(),
            leading_terminal: true,
            span: None,
            branch_local_cardinality: ChildCardinality::ONE,
            group_local_cardinality: ChildCardinality::ONE,
        };
        let _ = target_rule;
        return translate_element_read(&element, usize::MAX, suffix, ctx, body);
    }
    if ctx.token_types.contains_key(name) {
        let element = ElementRef {
            label: None,
            target: name.to_owned(),
            token_types: vec![ctx.token_types[name]],
            is_block: false,
            is_list: false,
            cardinality: ChildCardinality::ONE,
            stable_accessor: false,
            choice_branch: Vec::new(),
            choice_arity: Vec::new(),
            choice_spans: Vec::new(),
            group_spans: Vec::new(),
            branch_spans: Vec::new(),
            leading_terminal: true,
            span: None,
            branch_local_cardinality: ChildCardinality::ONE,
            group_local_cardinality: ChildCardinality::ONE,
        };
        return translate_element_read(&element, usize::MAX, suffix, ctx, body);
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!("cannot translate ${name} in embedded action: {body}"),
    ))
}

/// `$text` — text matched so far for the current rule.
pub(crate) fn text_expression(ctx: &TranslationCtx<'_>) -> String {
    match ctx.site {
        ActionSite::Body => {
            "self.base.text_interval(action.start_index(), action.stop_index())".to_owned()
        }
        ActionSite::After | ActionSite::Init => {
            "{ let __stop = self.base.rule_stop_token_index(antlr4_runtime::IntStream::index(self.base.input()), __consumed_eof); self.base.text_interval(__rule_start, __stop) }"
                .to_owned()
        }
    }
}

/// `$ctx.member` — a labeled element read (`$ctx.r`) or a generated child
/// iterator (`$ctx.elseIfStatement_children()`).
pub(crate) fn translate_ctx_member(
    member: &str,
    ctx: &TranslationCtx<'_>,
    body: &str,
) -> io::Result<String> {
    if let Some((element, occurrence)) = ctx.resolve_label(member) {
        // `$ctx.r` denotes the labeled child's subtree (Java field of the
        // context); translate like `$r.ctx`.
        return translate_element_read(&element, occurrence, Some("ctx"), ctx, body);
    }
    if let Some(rule_name) = member.strip_suffix("_children") {
        if let Some(rule_index) = ctx.rule_index_by_name(rule_name) {
            return Ok(format!(
                "__ctx.child_rules(self.base.parse_tree_storage(), self.base.token_store(), {rule_index})"
            ));
        }
    }
    if let Some(rule_name) = member.strip_suffix("_all") {
        if let Some(rule_index) = ctx.rule_index_by_name(rule_name) {
            return Ok(format!(
                "__ctx.child_rules(self.base.parse_tree_storage(), self.base.token_store(), {rule_index}).collect::<Vec<_>>()"
            ));
        }
    }
    if ctx.rule().attr(member).is_some() {
        return Ok(format!("__attrs.{}", escape_keyword(member)));
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!("cannot translate $ctx.{member} in embedded action: {body}"),
    ))
}

/// Reads a rule/token element reference with an optional attribute suffix.
///
/// `occurrence == usize::MAX` means "implicit reference": ANTLR resolves
/// `$e` to the most recent `e` match, i.e. the LAST matching child so far.
pub(crate) fn translate_element_read(
    element: &ElementRef,
    occurrence: usize,
    suffix: Option<&str>,
    ctx: &TranslationCtx<'_>,
    body: &str,
) -> io::Result<String> {
    if element.is_list {
        // `label+=x`: expose the matching children as a lazy Rust iterator.
        if let Some(rule_index) = ctx.rule_index_by_name(&element.target) {
            return match suffix {
                None | Some("ctx") => Ok(format!(
                    "__ctx.child_rule_trees(self.base.parse_tree_storage(), self.base.token_store(), {rule_index})"
                )),
                Some(other) => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unsupported list-label read .{other} in embedded action: {body}"),
                )),
            };
        }
        if let Some(token_type) = ctx.token_types.get(&element.target) {
            return Ok(format!(
                "__ctx.child_tokens(self.base.parse_tree_storage(), self.base.token_store(), {token_type})"
            ));
        }
        // A list label whose target names neither a rule nor a token type has no
        // iterator form: the block read below picks *one* terminal and renders it as
        // a `String`, so falling through emitted `.collect()` on a `String` for
        // `xs+='a'` (a literal is not a `token_types` key). Decline instead of
        // generating code that does not compile.
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "cannot translate list label ${} in embedded action: {body}",
                element.label.as_deref().unwrap_or_default()
            ),
        ));
    }
    if element.is_block {
        // A labeled `(...)` block over tokens: `$myset.stop` / `$myset.text` read
        // the token the block matched. A bare `$myset` read denotes the Token
        // object itself (Java prints `Token.toString()`), the same rendering as
        // start/stop.
        //
        // The block has no target to query, so the read walks the context's
        // terminal children and picks by *position*. It uses the *labeled* iterator,
        // which skips deleted-token errors while keeping inserted missing ones —
        // a grammar-derived index knows nothing about recovery, and a deleted token
        // would otherwise shift every later position (see #235 for the same rule on
        // token accessors): `occurrence` is the number of
        // terminals matched ahead of the block on this parse path. `usize::MAX`
        // means the position is not fixed, in which case the most recent terminal
        // is the best available answer — the historical behaviour.
        let pick = if occurrence == usize::MAX {
            "last()".to_owned()
        } else {
            format!("nth({occurrence})")
        };
        return match suffix {
            None | Some("stop" | "start") => Ok(format!(
                "__ctx.labeled_terminal_children(self.base.parse_tree_storage(), self.base.token_store()).{pick}.map(|__t| __t.symbol().to_string()).unwrap_or_default()"
            )),
            Some("text") => Ok(format!(
                "__ctx.labeled_terminal_children(self.base.parse_tree_storage(), self.base.token_store()).{pick}.map(|__t| __t.text().to_owned()).unwrap_or_default()"
            )),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported block-label read in embedded action: {body}"),
            )),
        };
    }
    if let Some(rule_index) = ctx.rule_index_by_name(&element.target) {
        let pick = if occurrence == usize::MAX {
            "last()".to_owned()
        } else {
            format!("nth({occurrence})")
        };
        return match suffix {
            Some("ctx") | None => Ok(format!(
                "__ctx.child_rule_trees(self.base.parse_tree_storage(), self.base.token_store(), {rule_index}).{pick}.expect(\"labeled rule child\")"
            )),
            Some("text") => Ok(format!(
                "__ctx.child_rules(self.base.parse_tree_storage(), self.base.token_store(), {rule_index}).{pick}.map(|__c| __c.text()).unwrap_or_default()"
            )),
            Some("start") => Ok(format!(
                "__ctx.child_rules(self.base.parse_tree_storage(), self.base.token_store(), {rule_index}).{pick}.and_then(|__c| __c.start()).map(|__t| __t.to_string()).unwrap_or_default()"
            )),
            Some("stop") => Ok(format!(
                "__ctx.child_rules(self.base.parse_tree_storage(), self.base.token_store(), {rule_index}).{pick}.and_then(|__c| __c.stop()).map(|__t| __t.to_string()).unwrap_or_default()"
            )),
            Some(attr) => {
                let target_rule = &ctx.model.rules[rule_index];
                let Some(decl) = target_rule.attr(attr) else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "rule {} has no attribute {attr} (embedded action: {body})",
                            element.target
                        ),
                    ));
                };
                let attrs_struct = attrs_struct_name(rule_index);
                let field = escape_keyword(&decl.name);
                Ok(format!(
                    "__ctx.child_rules(self.base.parse_tree_storage(), self.base.token_store(), {rule_index}).{pick}.and_then(|__c| __c.generated_attrs::<{attrs_struct}>()).map(|__a| __a.{field}.clone()).unwrap_or_default()"
                ))
            }
        };
    }
    if let Some(token_type) = ctx.token_types.get(&element.target) {
        let pick = if occurrence == usize::MAX {
            "last()".to_owned()
        } else {
            format!("nth({occurrence})")
        };
        return match suffix {
            Some("text") => Ok(format!(
                "__ctx.child_tokens(self.base.parse_tree_storage(), self.base.token_store(), {token_type}).{pick}.map(|__t| __t.text().to_owned()).unwrap_or_default()"
            )),
            Some("int") => Ok(format!(
                "__ctx.child_tokens(self.base.parse_tree_storage(), self.base.token_store(), {token_type}).{pick}.map(|__t| __t.text().parse::<i32>().unwrap_or_default()).unwrap_or_default()"
            )),
            Some("line") => Ok(format!(
                "__ctx.child_tokens(self.base.parse_tree_storage(), self.base.token_store(), {token_type}).{pick}.map(|__t| __t.symbol().line()).unwrap_or_default()"
            )),
            None | Some("stop" | "start") => Ok(format!(
                "__ctx.child_tokens(self.base.parse_tree_storage(), self.base.token_store(), {token_type}).{pick}.map(|__t| __t.symbol().to_string()).unwrap_or_default()"
            )),
            Some(other) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unsupported token attribute .{other} on ${} (embedded action: {body})",
                    element.target
                ),
            )),
        };
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "cannot resolve element ${} in embedded action: {body}",
            element.target
        ),
    ))
}

/// Escapes attribute names that collide with Rust keywords (`$return`).
pub(crate) fn escape_keyword(name: &str) -> String {
    match name {
        "return" | "type" | "match" | "loop" | "move" | "ref" | "self" | "super" | "box"
        | "const" | "continue" | "crate" | "else" | "enum" | "extern" | "fn" | "for" | "if"
        | "impl" | "in" | "let" | "mod" | "mut" | "pub" | "static" | "struct" | "trait"
        | "unsafe" | "use" | "where" | "while" => format!("r#{name}"),
        _ => name.to_owned(),
    }
}
