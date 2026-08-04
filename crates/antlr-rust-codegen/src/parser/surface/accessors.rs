#[derive(Clone, Debug, Eq, PartialEq)]
struct ContextLabelAccessor {
    source_name: String,
    target: String,
    token_types: Vec<i32>,
    cardinality: embedded::ChildCardinality,
    selector: ContextLabelSelector,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ContextLabelSelector {
    Nth(usize),
    LastAfter(usize),
    AllAfter(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ContextLabelSelection {
    pub(crate) preferred: ContextLabelSelector,
    /// A last-match selector that is equivalent to `preferred`, when one exists.
    pub(crate) compatible_last_after: Option<usize>,
}

pub(crate) fn reconcile_context_label_selections(
    selections: &[ContextLabelSelection],
) -> Option<ContextLabelSelector> {
    let first = selections.first()?;
    if selections
        .iter()
        .all(|selection| selection.preferred == first.preferred)
    {
        return Some(first.preferred);
    }
    let skip = first.compatible_last_after?;
    selections
        .iter()
        .all(|selection| selection.compatible_last_after == Some(skip))
        .then_some(ContextLabelSelector::LastAfter(skip))
}

fn context_label_accessors(
    rule: &embedded::RuleModel,
    alternative_label: Option<&str>,
) -> Vec<ContextLabelAccessor> {
    let alternatives = context_alternatives(rule, alternative_label);
    let labels = alternatives
        .iter()
        .flat_map(|alternative| {
            alternative
                .refs
                .iter()
                .filter_map(|element| element.label.clone())
        })
        .collect::<BTreeSet<_>>();
    labels
        .into_iter()
        .filter_map(|label| context_label_accessor(&alternatives, label))
        .collect()
}

fn context_label_accessor(
    alternatives: &[&embedded::AltModel],
    label: String,
) -> Option<ContextLabelAccessor> {
    let declarations = alternatives
        .iter()
        .flat_map(|alternative| alternative.refs.iter())
        .filter(|element| element.label.as_deref() == Some(label.as_str()))
        .collect::<Vec<_>>();
    let first = declarations.first()?;
    // Token-backed declarations may carry different token sets per alternative
    // (`r x=(A|B) r | r x=(C|D) r`); ANTLR still declares a single token field
    // for the label, so union the sets and let the per-alternative selector
    // checks below reject layouts where one occurrence lookup cannot serve
    // every alternative. Rule references keep requiring one shared target.
    let union_token_sets = declarations
        .iter()
        .all(|element| !element.token_types.is_empty());
    if (first.target.is_empty() && first.token_types.is_empty())
        || declarations.iter().any(|element| {
            !element.stable_accessor
                || element.is_list != first.is_list
                || !(union_token_sets || same_context_ref_target(element, first))
        })
    {
        return None;
    }

    let is_list = first.is_list;
    let reference = embedded::ElementRef {
        label: None,
        // Only read for rule-backed labels (rendering gates on empty
        // `token_types`), where the guard above already forced every
        // declaration onto one shared target.
        target: first.target.clone(),
        token_types: declarations
            .iter()
            .flat_map(|element| element.token_types.iter().copied())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        is_block: first.is_block,
        is_list,
        cardinality: embedded::ChildCardinality::ONE,
        stable_accessor: true,
        choice_branch: Vec::new(),
        choice_arity: Vec::new(),
        choice_spans: Vec::new(),
        group_spans: Vec::new(),
        branch_spans: Vec::new(),
        leading_terminal: true,
        span: None,
        branch_local_cardinality: embedded::ChildCardinality::ONE,
        group_local_cardinality: embedded::ChildCardinality::ONE,
    };
    let mut selections = Vec::with_capacity(alternatives.len());
    let mut cardinalities = Vec::with_capacity(alternatives.len());
    for alternative in alternatives {
        let matching = alternative
            .refs
            .iter()
            .enumerate()
            .filter(|(_, element)| element.label.as_deref() == Some(label.as_str()))
            .collect::<Vec<_>>();
        if matching.is_empty() {
            let target_cardinality = sum_child_cardinalities(
                alternative
                    .refs
                    .iter()
                    .filter(|element| context_ref_can_match_target(element, &reference))
                    .map(|element| element.cardinality),
            );
            if target_cardinality.max != Some(0) {
                return None;
            }
            cardinalities.push(embedded::ChildCardinality::ZERO);
            continue;
        }

        let alternative_selection =
            context_label_selector(alternative, &matching, &label, &reference, is_list)?;
        selections.push(alternative_selection);
        cardinalities.push(if is_list {
            sum_child_cardinalities(matching.iter().map(|(_, element)| element.cardinality))
        } else {
            matching[0].1.cardinality
        });
    }

    let selector = reconcile_context_label_selections(&selections)?;
    let mut cardinality = choice_cardinality(&cardinalities);
    if !is_list {
        cardinality = embedded::ChildCardinality {
            min: usize::from(cardinality.min > 0),
            max: Some(usize::from(cardinality.max != Some(0))),
        };
    }
    Some(ContextLabelAccessor {
        source_name: label,
        target: reference.target,
        token_types: reference.token_types,
        cardinality,
        selector,
    })
}

fn context_label_selector(
    alternative: &embedded::AltModel,
    matching: &[(usize, &embedded::ElementRef)],
    label: &str,
    target: &embedded::ElementRef,
    is_list: bool,
) -> Option<ContextLabelSelection> {
    let first_position = matching[0].0;
    let labeled = matching[0].1;
    // A sibling branch that matches the same target supplies a child at the very
    // position this accessor reads, but on a parse where the label is unset —
    // `(left=unary | right=unary)` would let `right()` return `left`'s child.
    // Positional lookup cannot tell them apart, so decline.
    // Another *declaration* of the same label is not an impostor — it binds the
    // label too, and `context_label_accessor` already proved the declarations share
    // one read. Only an unlabeled (or differently-labeled) sibling child can be
    // mistaken for this label's.
    let start =
        exact_target_cardinality_on_path(&alternative.refs[..first_position], target, labeled)?;
    // A sibling branch only collides when it can actually put a matching child at the
    // position this accessor reads. `(A | A A x=A)` selects occurrence 2 while the
    // sibling branch supplies at most one `A`, so `nth(2)` is safely empty there.
    let sibling_supplies_target = alternative.refs.iter().any(|element| {
        if element.label.as_deref() == Some(label)
            || element.can_coexist_with(labeled)
            || !context_ref_can_match_target(element, target)
            || element.cardinality.max == Some(0)
        {
            return false;
        }
        let position = alternative
            .refs
            .iter()
            .position(|candidate| std::ptr::eq(candidate, element));
        let reach = position.and_then(|position| {
            let before =
                exact_target_cardinality_on_path(&alternative.refs[..position], target, element)?;
            // The highest occurrence this ref can occupy on its own path.
            element
                .cardinality
                .max
                .map(|max| before.saturating_add(max))
        });
        reach.is_none_or(|reach| reach > start)
    });
    if sibling_supplies_target {
        return None;
    }
    if is_list {
        let has_unlabeled_target = alternative.refs[first_position..].iter().any(|element| {
            context_ref_can_match_target(element, target)
                && element.cardinality.max != Some(0)
                && element.label.as_deref() != Some(label)
        });
        // A same-target ref *before* the label normally just shifts `start`, but if
        // the two share a repeated group it recurs on every iteration and interleaves
        // with the labeled children: `(A xs+=A)+` skips one `A` and then collects the
        // second iteration's unlabeled prefix too. No `skip` can separate them.
        let repeats_with_prefix = alternative.refs[..first_position].iter().any(|element| {
            context_ref_can_match_target(element, target)
                && element.cardinality.max != Some(0)
                && element.label.as_deref() != Some(label)
                && element.group_spans.iter().any(|group| {
                    // Shared and repeatable: `max` is not one, so the group can run
                    // more than once.
                    labeled.group_spans.contains(group)
                        && !matches!(element.cardinality.max, Some(0 | 1))
                })
        });
        if repeats_with_prefix {
            return None;
        }
        // `AllAfter(start)` skips `start` children then takes the rest, so repeated
        // declarations on one path are fine — the later ones fall inside the tail
        // (`xs+=e (op xs+=e)*` skips 0 and collects every `e`). What it cannot serve
        // is *mutually exclusive* declarations that begin at different offsets:
        // `(A xs+=A | xs+=A)` needs skip 1 on one branch and 0 on the other.
        let starts_agree = matching.iter().all(|(position, declaration)| {
            if declaration.can_coexist_with(labeled) {
                return true;
            }
            exact_target_cardinality_on_path(&alternative.refs[..*position], target, declaration)
                == Some(start)
        });
        return (!has_unlabeled_target && starts_agree).then_some(ContextLabelSelection {
            preferred: ContextLabelSelector::AllAfter(start),
            compatible_last_after: None,
        });
    }
    // Several declarations can share one positional read when they are mutually
    // exclusive and each sits at the same occurrence — `(x=A | x=B)` binds exactly
    // one token on every parse, and the accessor's unioned token set selects it.
    if matching.len() != 1 {
        let mutually_exclusive = matching.iter().enumerate().all(|(index, (_, left))| {
            matching[index + 1..]
                .iter()
                .all(|(_, right)| !left.can_coexist_with(right))
        });
        let positions = matching
            .iter()
            .map(|(position, element)| {
                exact_target_cardinality_on_path(&alternative.refs[..*position], target, element)
            })
            .collect::<Option<Vec<_>>>()?;
        let agreed = positions.first().copied()?;
        if !mutually_exclusive || positions.iter().any(|position| *position != agreed) {
            return None;
        }
        // Each declaration must also survive the single-label hazards: an *optional*
        // one can be displaced by a following same-target child sliding into its
        // slot, and the shared read cannot tell them apart either
        // (`(x=A? B | x=B)` returns the unlabeled `B` when `x` is absent).
        for (position, declaration) in matching {
            // Optionality here means the *declaration's own* EBNF suffix — a `min: 0`
            // that only reflects its branch possibly not being taken does not make it
            // displaceable, since the read is chosen per branch anyway.
            if declaration.branch_local_cardinality.min == 0
                && alternative.refs[position + 1..].iter().any(|following| {
                    context_ref_can_match_target(following, target)
                        && following.cardinality.max != Some(0)
                        && following.can_coexist_with(declaration)
                })
            {
                return None;
            }
        }
        // A *repeated* scalar declaration (`x=A+`) is overwritten each iteration, so
        // ANTLR exposes the last match — `nth` would pin the first. But `LastAfter`
        // applies to every branch, so it is only sound when no branch has a matching
        // child *after* its declaration: in `(x=A A B | x=A+ C)` the first branch's
        // trailing unlabeled `A` would become the `last()`.
        let followed = matching.iter().any(|(position, declaration)| {
            alternative.refs[position + 1..].iter().any(|following| {
                following.label.as_deref() != Some(label)
                    && context_ref_can_match_target(following, target)
                    && following.cardinality.max != Some(0)
                    && following.can_coexist_with(declaration)
            })
        });
        if matching
            .iter()
            .any(|(_, element)| element.cardinality.is_repeated())
        {
            if followed {
                return None;
            }
            return Some(ContextLabelSelection {
                preferred: ContextLabelSelector::LastAfter(agreed),
                compatible_last_after: Some(agreed),
            });
        }
        return Some(ContextLabelSelection {
            preferred: ContextLabelSelector::Nth(agreed),
            compatible_last_after: (!followed).then_some(agreed),
        });
    }
    let element = matching[0].1;
    if !element.cardinality.is_repeated() {
        // An optional labeled occurrence (`x=A? C` with `C` in the accessor's
        // token set) must not be shadowed by a following match sliding into
        // its position when it is absent.
        let shadowed_when_absent = element.cardinality.min == 0
            && alternative.refs[first_position + 1..]
                .iter()
                .any(|following| {
                    context_ref_can_match_target(following, target)
                        && following.cardinality.max != Some(0)
                        && following_can_shadow_absent_label(element, following)
                });
        if shadowed_when_absent {
            return None;
        }
        let followed = has_following_context_target(alternative, first_position, target);
        return Some(ContextLabelSelection {
            preferred: ContextLabelSelector::Nth(start),
            compatible_last_after: (!followed).then_some(start),
        });
    }
    let has_following_target = has_following_context_target(alternative, first_position, target);
    (!has_following_target).then_some(ContextLabelSelection {
        preferred: ContextLabelSelector::LastAfter(start),
        compatible_last_after: Some(start),
    })
}

fn has_following_context_target(
    alternative: &embedded::AltModel,
    position: usize,
    target: &embedded::ElementRef,
) -> bool {
    alternative.refs[position + 1..].iter().any(|following| {
        context_ref_can_match_target(following, target) && following.cardinality.max != Some(0)
    })
}

fn following_can_shadow_absent_label(
    label: &embedded::ElementRef,
    following: &embedded::ElementRef,
) -> bool {
    // A direct suffix can omit the label while every enclosing block remains
    // present, so no following ref is coupled to that absence.
    if label.group_local_cardinality.min == 0 {
        return true;
    }
    // Otherwise the label is optional only because an enclosing group can be
    // absent or an enclosing choice can take another branch. A following ref
    // under every such group and branch disappears with the label and cannot
    // slide into its occurrence.
    label
        .group_spans
        .iter()
        .filter(|group| group.optional)
        .any(|group| !following.group_spans.contains(group))
        || label
            .choice_branch
            .iter()
            .any(|branch| !following.choice_branch.contains(branch))
}

fn same_context_ref_target(left: &embedded::ElementRef, right: &embedded::ElementRef) -> bool {
    if left.token_types.is_empty() && right.token_types.is_empty() {
        left.target == right.target
    } else {
        left.token_types == right.token_types
    }
}

fn context_ref_can_match_target(
    element: &embedded::ElementRef,
    target: &embedded::ElementRef,
) -> bool {
    if element.token_types.is_empty() || target.token_types.is_empty() {
        return same_context_ref_target(element, target);
    }
    element
        .token_types
        .iter()
        .any(|token_type| target.token_types.contains(token_type))
}

/// Number of `target`-matching children that precede a ref *on the same parse
/// path* as `path`, or `None` when that number is not fixed.
fn exact_target_cardinality_on_path(
    refs: &[embedded::ElementRef],
    target: &embedded::ElementRef,
    path: &embedded::ElementRef,
) -> Option<usize> {
    // Filtering to one path and then demanding cross-branch agreement are mutually
    // exclusive: the other branches were removed deliberately, so requiring them to
    // contribute would reject `(A x=A | B)`, where the retained `A` still carries
    // the choice's full arity of two.
    // A ref in a branch `path` cannot reach never precedes it, so drop those
    // before counting: in `(left=unary | right=unary)` the `left` ref must not
    // shift `right` to occurrence 1.
    let reachable = refs
        .iter()
        .filter(|element| element.can_coexist_with(path))
        .cloned()
        .map(|mut element| {
            // Drop the branch tags shared with `path`: on this path those branches
            // are taken, so their refs count as plain sequential children rather
            // than alternatives awaiting cross-branch agreement.
            element.retain_choices(|choice| {
                !path.choice_branch.iter().any(|&(taken, _)| taken == choice)
            });
            // Their cardinality on this path is the branch-local one — and when the
            // ref shares every optional group with the label, those groups are taken
            // wherever the label is bound, so even their quantifiers are satisfied.
            // `(A x=A)? EOF` has exactly one `A` before the label on every parse that
            // binds it, though both figures otherwise report `0..1` from the `?`.
            //
            // A *repeated* group not shared with the label is the exception: knowing
            // it ran says nothing about how many times, so its contribution stays
            // unfixed. `((A B)+ x=A)?` has a variable run of `A` ahead of the label
            // even though the outer `?` is satisfied. This mirrors `on_taken_group`
            // in the action-resolution path.
            let closed_repeated_group = element
                .group_spans
                .iter()
                .any(|group| group.repeated && !path.group_spans.contains(group));
            let shares_optional_groups = !element.group_spans.is_empty()
                && !closed_repeated_group
                && element
                    .group_spans
                    .iter()
                    .filter(|group| group.optional)
                    .all(|group| path.group_spans.contains(group));
            let on_path = if shares_optional_groups {
                element.group_local_cardinality
            } else {
                element.branch_local_cardinality
            };
            element.cardinality = on_path;
            // `exact_target_cardinality` judges exactness from the branch-local
            // figure, so it has to see the same value.
            element.branch_local_cardinality = on_path;
            element
        })
        .collect::<Vec<_>>();
    exact_target_cardinality(&reachable, target)
}

fn exact_target_cardinality(
    refs: &[embedded::ElementRef],
    target: &embedded::ElementRef,
) -> Option<usize> {
    // Refs are grouped by their innermost choice so that an *exhaustive* choice
    // counts once rather than per branch. `(a=A | b=A) x=A` always contributes
    // exactly one `A` before `x`, even though each branch ref alone reports
    // `0..1`; summing them independently would read as inexact and drop `x()`.
    let mut total = 0_usize;
    let mut choice_totals: BTreeMap<(usize, usize), Option<usize>> = BTreeMap::new();
    for element in refs {
        if !context_ref_can_match_target(element, target) {
            continue;
        }
        if !element.token_types.is_empty()
            && !element
                .token_types
                .iter()
                .all(|token_type| target.token_types.contains(token_type))
        {
            return None;
        }
        let exact = element.cardinality.max?;
        // A ref inside a choice reports `min: 0` because its *branch* may not be
        // taken, but within that branch it contributes its branch-local count
        // exactly. Judge exactness against that, so an *optional* choice
        // (`(a=A | b=A)? x=A`) stays inexact — the group may yield nothing.
        let local = element.branch_local_cardinality;
        let contribution = (local.min == exact && local.max == Some(exact)).then_some(exact);
        match element.choice_branch.last() {
            // Sequential: its count adds directly.
            None => total = total.saturating_add(contribution?),
            // Inside a choice: accumulate per branch, compare branches after.
            Some(&key) => {
                let branch = choice_totals.entry(key).or_insert(Some(0));
                *branch = match (*branch, contribution) {
                    (Some(sum), Some(next)) => Some(sum.saturating_add(next)),
                    _ => None,
                };
            }
        }
    }
    // A choice is exact only when every one of its branches contributes the same
    // count. Branches with no matching ref never entered the map above yet still
    // contribute zero, so the full branch set is recovered from `refs`.
    //
    // Nested choices are folded innermost-first: once an inner choice agrees, its
    // count is attributed to the *enclosing* branch that contains it, so
    // `((a=A | b=A) | c=A) x=A` — where every path yields one `A` — stays exact.
    // Arity comes from the recorded `choice_arity`, not from the branches seen: an
    // *empty* alternative emits no ref, so `(a=A | )` would otherwise look like a
    // one-branch choice that always yields an `A`.
    let mut arity_of_choice: BTreeMap<usize, usize> = BTreeMap::new();
    for element in refs {
        for (&(choice, _), &arity) in element.choice_branch.iter().zip(&element.choice_arity) {
            arity_of_choice.insert(choice, arity);
        }
    }
    let mut branches_per_choice: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    let mut depth_of_choice: BTreeMap<usize, usize> = BTreeMap::new();
    for element in refs {
        for (depth, &(choice, branch)) in element.choice_branch.iter().enumerate() {
            branches_per_choice
                .entry(choice)
                .or_default()
                .insert(branch);
            depth_of_choice.insert(choice, depth);
        }
    }
    // Ancestry per branch key, so an inner choice's total can be re-attributed.
    let mut ancestry: BTreeMap<(usize, usize), Vec<(usize, usize)>> = BTreeMap::new();
    for element in refs {
        if let Some(&key) = element.choice_branch.last() {
            ancestry.insert(key, element.choice_branch.clone());
        }
    }
    let mut pending = choice_totals;
    // Deepest choices first, so inner results roll up into their parents.
    let mut choices = depth_of_choice
        .iter()
        .map(|(c, d)| (*d, *c))
        .collect::<Vec<_>>();
    choices.sort_unstable_by_key(|(depth, _)| std::cmp::Reverse(*depth));
    for (_, choice) in choices {
        let counts = pending
            .iter()
            .filter(|((candidate, _), _)| *candidate == choice)
            .map(|((_, branch), count)| (*branch, *count))
            .collect::<Vec<_>>();
        if counts.is_empty() {
            continue;
        }
        let expected = arity_of_choice
            .get(&choice)
            .copied()
            .unwrap_or_else(|| branches_per_choice.get(&choice).map_or(0, BTreeSet::len));
        let first = counts.first().and_then(|(_, count)| *count)?;
        if counts.len() != expected || counts.iter().any(|(_, count)| *count != Some(first)) {
            return None;
        }
        for (branch, _) in &counts {
            pending.remove(&(choice, *branch));
        }
        // Attribute this choice's agreed count to its own enclosing branch, if any.
        let parent = counts.first().and_then(|(branch, _)| {
            ancestry
                .get(&(choice, *branch))
                .and_then(|chain| chain.split_last().map(|(_, rest)| rest.last().copied()))
                .flatten()
        });
        match parent {
            Some(parent_key) => {
                let slot = pending.entry(parent_key).or_insert(Some(0));
                *slot = slot.map(|sum| sum.saturating_add(first));
            }
            None => total = total.saturating_add(first),
        }
    }
    Some(total)
}

fn sum_child_cardinalities(
    cardinalities: impl IntoIterator<Item = embedded::ChildCardinality>,
) -> embedded::ChildCardinality {
    cardinalities.into_iter().fold(
        embedded::ChildCardinality::ZERO,
        |mut total, cardinality| {
            total.min = total.min.saturating_add(cardinality.min);
            total.max = match (total.max, cardinality.max) {
                (Some(current), Some(next)) => Some(current.saturating_add(next)),
                _ => None,
            };
            total
        },
    )
}

fn choice_cardinality(alternatives: &[embedded::ChildCardinality]) -> embedded::ChildCardinality {
    let mut min = usize::MAX;
    let mut max = Some(0_usize);
    for cardinality in alternatives {
        min = min.min(cardinality.min);
        max = match (max, cardinality.max) {
            (Some(current), Some(next)) => Some(current.max(next)),
            _ => None,
        };
    }
    embedded::ChildCardinality {
        min: if min == usize::MAX { 0 } else { min },
        max,
    }
}

pub(crate) fn allocate_context_method(
    preferred: String,
    fallback_stem: &str,
    used: &mut BTreeSet<String>,
) -> String {
    if used.insert(preferred.clone()) {
        return preferred;
    }
    allocate_numbered_listener_method(fallback_stem, used)
}

fn accessor_stem(name: &str) -> String {
    rust_function_name(name).trim_start_matches("r#").to_owned()
}

#[derive(Debug, Default, Eq, PartialEq)]
struct RenderedContextAccessors {
    recovered: String,
    validated: String,
    validation: String,
    compatibility: String,
}

fn render_required_accessor_validation(
    out: &mut String,
    method: &str,
    validation_error_name: &str,
) {
    let _ = writeln!(
        out,
        "        let _ = context.{method}().map_err({validation_error_name}::MissingChild)?;"
    );
}

fn render_repeated_accessor_validation(
    out: &mut String,
    method: &str,
    view_name: &str,
    child_name: &str,
    minimum: usize,
    validation_error_name: &str,
) {
    if minimum == 0 {
        return;
    }
    let _ = writeln!(
        out,
        "        {{\n            let actual = context.{method}().count();\n            if actual < {minimum} {{\n                return Err({validation_error_name}::InvalidChildCount {{\n                    context: \"{view_name}\",\n                    child: \"{child_name}\",\n                    minimum: {minimum},\n                    actual,\n                }});\n            }}\n        }}"
    );
}

fn antlr4rust_compat_method_name(source_name: &str) -> String {
    rust_identifier(source_name)
}

pub(crate) fn render_antlr4rust_rule_all_accessor(
    out: &mut String,
    source_name: &str,
    native_method: &str,
    child_view: &str,
    context_wrapper: &str,
    emitted_methods: &mut BTreeSet<String>,
) {
    let method = antlr4rust_compat_method_name(&format!("{source_name}_all"));
    if !emitted_methods.insert(method.clone()) {
        return;
    }
    let _ = writeln!(
        out,
        "    #[allow(non_snake_case)]\n    pub fn {method}(&self) -> Vec<{context_wrapper}<{child_view}<'a>>> {{\n        self.0.{native_method}().map({context_wrapper}).collect()\n    }}"
    );
}

fn render_antlr4rust_indexed_rule_accessor(
    out: &mut String,
    source_name: &str,
    native_method: &str,
    child_view: &str,
    context_wrapper: &str,
    emitted_methods: &mut BTreeSet<String>,
) {
    let method = antlr4rust_compat_method_name(source_name);
    if !emitted_methods.insert(method.clone()) {
        return;
    }
    let _ = writeln!(
        out,
        "    #[allow(non_snake_case)]\n    pub fn {method}(&self, index: usize) -> Option<{context_wrapper}<{child_view}<'a>>> {{\n        self.0.{native_method}().nth(index).map({context_wrapper})\n    }}"
    );
}

#[derive(Clone, Copy)]
pub(crate) struct Antlr4RustSingleRuleAccessorRender<'a> {
    pub(crate) source_name: &'a str,
    pub(crate) native_method: &'a str,
    pub(crate) child_view: &'a str,
    pub(crate) required: bool,
    pub(crate) context_wrapper: &'a str,
}

pub(crate) fn render_antlr4rust_single_rule_accessor(
    out: &mut String,
    context: Antlr4RustSingleRuleAccessorRender<'_>,
    emitted_methods: &mut BTreeSet<String>,
) {
    let Antlr4RustSingleRuleAccessorRender {
        source_name,
        native_method,
        child_view,
        required,
        context_wrapper,
    } = context;
    let method = antlr4rust_compat_method_name(source_name);
    if !emitted_methods.insert(method.clone()) {
        return;
    }
    let recover_missing = if required { ".ok()" } else { "" };
    let _ = writeln!(
        out,
        "    #[allow(non_snake_case)]\n    pub fn {method}(&self) -> Option<{context_wrapper}<{child_view}<'a>>> {{\n        self.0.{native_method}(){recover_missing}.map({context_wrapper})\n    }}"
    );
}

pub(crate) fn render_antlr4rust_single_token_accessor(
    out: &mut String,
    token_name: &str,
    native_method: &str,
    required: bool,
    emitted_methods: &mut BTreeSet<String>,
) {
    let method = antlr4rust_compat_method_name(token_name);
    if !emitted_methods.insert(method.clone()) {
        return;
    }
    let recover_missing = if required { ".ok()" } else { "" };
    let _ = writeln!(
        out,
        "    #[allow(non_snake_case)]\n    pub fn {method}(&self) -> Option<TerminalNode<'a>> {{\n        self.0.{native_method}(){recover_missing}\n    }}"
    );
}

fn render_antlr4rust_indexed_token_accessor(
    out: &mut String,
    token_name: &str,
    native_method: &str,
    emitted_methods: &mut BTreeSet<String>,
) {
    let method = antlr4rust_compat_method_name(token_name);
    if !emitted_methods.insert(method.clone()) {
        return;
    }
    let _ = writeln!(
        out,
        "    #[allow(non_snake_case)]\n    pub fn {method}(&self, index: usize) -> Option<TerminalNode<'a>> {{\n        self.0.{native_method}().nth(index)\n    }}"
    );
}

pub(crate) fn render_antlr4rust_token_all_accessor(
    out: &mut String,
    token_name: &str,
    native_method: &str,
    emitted_methods: &mut BTreeSet<String>,
) {
    let method = antlr4rust_compat_method_name(&format!("{token_name}_all"));
    if !emitted_methods.insert(method.clone()) {
        return;
    }
    let _ = writeln!(
        out,
        "    #[allow(non_snake_case)]\n    pub fn {method}(&self) -> Vec<TerminalNode<'a>> {{\n        self.0.{native_method}().collect()\n    }}"
    );
}

pub(crate) fn antlr4rust_compat_method_names(
    view_name: &str,
    model: &embedded::EmbeddedModel,
    token_accessors: &[(String, i32)],
    child_cardinalities: &BTreeMap<String, embedded::ChildCardinality>,
) -> io::Result<BTreeSet<String>> {
    let mut methods = BTreeMap::new();
    let mut register = |method: String, source: String| -> io::Result<()> {
        if let Some(previous) = methods.insert(method.clone(), source.clone()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "antlr4rust compatibility accessor `{method}` in {view_name} is ambiguous \
                     between {previous} and {source}"
                ),
            ));
        }
        Ok(())
    };
    for child in &model.rules {
        let Some(cardinality) = child_cardinalities.get(child.name.as_str()) else {
            continue;
        };
        let method = if cardinality.is_repeated() {
            antlr4rust_compat_method_name(&format!("{}_all", child.name))
        } else {
            antlr4rust_compat_method_name(&child.name)
        };
        register(method, format!("parser rule `{}`", child.name))?;
        if cardinality.is_repeated() {
            register(
                antlr4rust_compat_method_name(&child.name),
                format!("indexed parser rule `{}`", child.name),
            )?;
        }
    }
    for (token_name, _) in token_accessors {
        let Some(cardinality) = child_cardinalities.get(token_name.as_str()) else {
            continue;
        };
        let method = if cardinality.is_repeated() {
            antlr4rust_compat_method_name(&format!("{token_name}_all"))
        } else {
            antlr4rust_compat_method_name(token_name)
        };
        register(method, format!("token `{token_name}`"))?;
        if cardinality.is_repeated() {
            register(
                antlr4rust_compat_method_name(token_name),
                format!("indexed token `{token_name}`"),
            )?;
        }
    }
    Ok(methods.into_keys().collect())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ContextCommonMethodNames {
    pub(crate) child_count: String,
    pub(crate) direct_terminals: String,
    pub(crate) rule_node: String,
    pub(crate) start: String,
    pub(crate) text: String,
}

pub(crate) fn context_common_method_names(
    compatibility_methods: &BTreeSet<String>,
) -> ContextCommonMethodNames {
    let mut used = compatibility_methods.clone();
    ContextCommonMethodNames {
        child_count: allocate_context_method(
            "child_count".to_owned(),
            "context_child_count",
            &mut used,
        ),
        direct_terminals: allocate_context_method(
            "direct_terminals".to_owned(),
            "context_direct_terminals",
            &mut used,
        ),
        rule_node: allocate_context_method("rule_node".to_owned(), "context_rule_node", &mut used),
        start: allocate_context_method("start".to_owned(), "context_start", &mut used),
        text: allocate_context_method("text".to_owned(), "context_text", &mut used),
    }
}

#[derive(Clone, Copy)]
struct RuleLabelAccessorRender<'a> {
    method: &'a str,
    view_name: &'a str,
    child_view: &'a str,
    child_index: usize,
    label: &'a ContextLabelAccessor,
    validation_error_name: &'a str,
}

fn render_rule_label_accessor(
    rendered: &mut RenderedContextAccessors,
    context: RuleLabelAccessorRender<'_>,
) {
    let RuleLabelAccessorRender {
        method,
        view_name,
        child_view,
        child_index,
        label,
        validation_error_name,
    } = context;
    if let ContextLabelSelector::AllAfter(skip) = label.selector {
        let _ = writeln!(
            rendered.recovered,
            "    pub fn {method}(&self) -> impl Iterator<Item = {child_view}<'a>> + '_ {{\n        __rule_children(self.__node, {child_index})\n            .skip({skip})\n            .map(move |node| {child_view}::__from_child_node(node, self.__invocation_states.as_deref()))\n    }}"
        );
        let _ = writeln!(
            rendered.validated,
            "    pub fn {method}(&self) -> impl Iterator<Item = {child_view}<'a, ValidatedTreeContext>> + '_ {{\n        __rule_children(self.__node, {child_index})\n            .skip({skip})\n            .map(move |node| {child_view}::<ValidatedTreeContext>::__from_validated_child_node(node, self.__invocation_states.as_deref()))\n    }}"
        );
        render_repeated_accessor_validation(
            &mut rendered.validation,
            method,
            view_name,
            &label.source_name,
            label.cardinality.min,
            validation_error_name,
        );
        return;
    }
    let lookup = match label.selector {
        ContextLabelSelector::Nth(occurrence) => format!(".nth({occurrence})"),
        ContextLabelSelector::LastAfter(skip) => format!(".skip({skip}).last()"),
        ContextLabelSelector::AllAfter(_) => unreachable!("handled above"),
    };
    if label.cardinality.is_required_single() {
        let _ = writeln!(
            rendered.recovered,
            "    pub fn {method}(&self) -> Result<{child_view}<'a>, MissingChildError> {{\n        __rule_children(self.__node, {child_index})\n            {lookup}\n            .map(|node| {child_view}::__from_child_node(node, self.__invocation_states.as_deref()))\n            .ok_or_else(|| MissingChildError::new(\"{view_name}\", \"{}\"))\n    }}",
            label.source_name
        );
        let _ = writeln!(
            rendered.validated,
            "    pub fn {method}(&self) -> {child_view}<'a, ValidatedTreeContext> {{\n        let Some(node) = __rule_children(self.__node, {child_index})\n            {lookup}\n        else {{\n            unreachable!(\"validated {view_name} is missing required child {}\")\n        }};\n        {child_view}::<ValidatedTreeContext>::__from_validated_child_node(\n            node,\n            self.__invocation_states.as_deref(),\n        )\n    }}",
            label.source_name
        );
        render_required_accessor_validation(
            &mut rendered.validation,
            method,
            validation_error_name,
        );
    } else {
        let _ = writeln!(
            rendered.recovered,
            "    pub fn {method}(&self) -> Option<{child_view}<'a>> {{\n        __rule_children(self.__node, {child_index})\n            {lookup}\n            .map(|node| {child_view}::__from_child_node(node, self.__invocation_states.as_deref()))\n    }}"
        );
        let _ = writeln!(
            rendered.validated,
            "    pub fn {method}(&self) -> Option<{child_view}<'a, ValidatedTreeContext>> {{\n        __rule_children(self.__node, {child_index})\n            {lookup}\n            .map(|node| {child_view}::<ValidatedTreeContext>::__from_validated_child_node(node, self.__invocation_states.as_deref()))\n    }}"
        );
    }
}

fn render_token_label_accessor(
    rendered: &mut RenderedContextAccessors,
    method: &str,
    view_name: &str,
    label: &ContextLabelAccessor,
    validation_error_name: &str,
) {
    let token_types = label
        .token_types
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let children = if let [token_type] = label.token_types.as_slice() {
        format!("__labeled_token_children(self.__node, {token_type})")
    } else {
        format!("__labeled_token_children_matching(self.__node, &[{token_types}])")
    };
    if let ContextLabelSelector::AllAfter(skip) = label.selector {
        let _ = writeln!(
            rendered.recovered,
            "    pub fn {method}(&self) -> impl Iterator<Item = TerminalNode<'a>> + '_ {{\n        {children}\n            .skip({skip})\n            .map(TerminalNode::new)\n    }}"
        );
        let _ = writeln!(
            rendered.validated,
            "    pub fn {method}(&self) -> impl Iterator<Item = TerminalNode<'a>> + '_ {{\n        {children}\n            .skip({skip})\n            .map(TerminalNode::new)\n    }}"
        );
        render_repeated_accessor_validation(
            &mut rendered.validation,
            method,
            view_name,
            &label.source_name,
            label.cardinality.min,
            validation_error_name,
        );
        return;
    }
    let lookup = match label.selector {
        ContextLabelSelector::Nth(occurrence) => format!(".nth({occurrence})"),
        ContextLabelSelector::LastAfter(skip) => format!(".skip({skip}).last()"),
        ContextLabelSelector::AllAfter(_) => unreachable!("handled above"),
    };
    if label.cardinality.is_required_single() {
        let _ = writeln!(
            rendered.recovered,
            "    pub fn {method}(&self) -> Result<TerminalNode<'a>, MissingChildError> {{\n        {children}\n            {lookup}\n            .map(TerminalNode::new)\n            .ok_or_else(|| MissingChildError::new(\"{view_name}\", \"{}\"))\n    }}",
            label.source_name
        );
        let _ = writeln!(
            rendered.validated,
            "    pub fn {method}(&self) -> TerminalNode<'a> {{\n        let Some(node) = {children}\n            {lookup}\n        else {{\n            unreachable!(\"validated {view_name} is missing required child {}\")\n        }};\n        TerminalNode::new(node)\n    }}",
            label.source_name
        );
        render_required_accessor_validation(
            &mut rendered.validation,
            method,
            validation_error_name,
        );
    } else {
        let _ = writeln!(
            rendered.recovered,
            "    pub fn {method}(&self) -> Option<TerminalNode<'a>> {{\n        {children}\n            {lookup}\n            .map(TerminalNode::new)\n    }}"
        );
        let _ = writeln!(
            rendered.validated,
            "    pub fn {method}(&self) -> Option<TerminalNode<'a>> {{\n        {children}\n            {lookup}\n            .map(TerminalNode::new)\n    }}"
        );
    }
}

#[derive(Clone, Copy)]
struct ContextAccessorsRender<'a> {
    view_name: &'a str,
    model: &'a embedded::EmbeddedModel,
    context_names: &'a ContextSurfaceNames,
    token_accessors: &'a [(String, i32)],
    child_cardinalities: &'a BTreeMap<String, embedded::ChildCardinality>,
    label_accessors: &'a [ContextLabelAccessor],
    validation_error_name: &'a str,
    antlr4rust_compat: bool,
    antlr4rust_context_wrapper: &'a str,
    common_methods: &'a ContextCommonMethodNames,
}

fn render_context_child_accessors(context: ContextAccessorsRender<'_>) -> RenderedContextAccessors {
    let ContextAccessorsRender {
        view_name,
        model,
        context_names,
        token_accessors,
        child_cardinalities,
        label_accessors,
        validation_error_name,
        antlr4rust_compat,
        antlr4rust_context_wrapper,
        common_methods,
    } = context;
    let mut rendered = RenderedContextAccessors::default();
    let mut used_methods = BTreeSet::from([
        common_methods.child_count.clone(),
        common_methods.direct_terminals.clone(),
        common_methods.rule_node.clone(),
        common_methods.start.clone(),
        common_methods.text.clone(),
    ]);
    let mut emitted_compatibility_methods = BTreeSet::new();
    for (child_index, child) in model
        .rules
        .iter()
        .enumerate()
        .filter(|(_, child)| child_cardinalities.contains_key(child.name.as_str()))
    {
        let cardinality = child_cardinalities[child.name.as_str()];
        let stem = accessor_stem(&child.name);
        let preferred = if cardinality.is_repeated() {
            format!("{stem}_children")
        } else {
            rust_function_name(&child.name)
        };
        let method =
            allocate_context_method(preferred, &format!("{stem}_rule_child"), &mut used_methods);
        let child_view = &context_names.rules[child_index].context_type;
        if cardinality.is_repeated() {
            let _ = writeln!(
                rendered.recovered,
                "    pub fn {method}(&self) -> impl Iterator<Item = {child_view}<'a>> + '_ {{\n        __rule_children(self.__node, {child_index})\n            .map(move |node| {child_view}::__from_child_node(node, self.__invocation_states.as_deref()))\n    }}"
            );
            let _ = writeln!(
                rendered.validated,
                "    pub fn {method}(&self) -> impl Iterator<Item = {child_view}<'a, ValidatedTreeContext>> + '_ {{\n        __rule_children(self.__node, {child_index})\n            .map(move |node| {child_view}::<ValidatedTreeContext>::__from_validated_child_node(node, self.__invocation_states.as_deref()))\n    }}"
            );
            render_repeated_accessor_validation(
                &mut rendered.validation,
                &method,
                view_name,
                &child.name,
                cardinality.min,
                validation_error_name,
            );
            if antlr4rust_compat {
                render_antlr4rust_indexed_rule_accessor(
                    &mut rendered.compatibility,
                    &child.name,
                    &method,
                    child_view,
                    antlr4rust_context_wrapper,
                    &mut emitted_compatibility_methods,
                );
                render_antlr4rust_rule_all_accessor(
                    &mut rendered.compatibility,
                    &child.name,
                    &method,
                    child_view,
                    antlr4rust_context_wrapper,
                    &mut emitted_compatibility_methods,
                );
            }
        } else if cardinality.is_required_single() {
            let _ = writeln!(
                rendered.recovered,
                "    pub fn {method}(&self) -> Result<{child_view}<'a>, MissingChildError> {{\n        __rule_children(self.__node, {child_index})\n            .next()\n            .map(|node| {child_view}::__from_child_node(node, self.__invocation_states.as_deref()))\n            .ok_or_else(|| MissingChildError::new(\"{view_name}\", \"{}\"))\n    }}",
                child.name
            );
            let _ = writeln!(
                rendered.validated,
                "    pub fn {method}(&self) -> {child_view}<'a, ValidatedTreeContext> {{\n        let Some(node) = __rule_children(self.__node, {child_index}).next() else {{\n            unreachable!(\"validated {view_name} is missing required child {}\")\n        }};\n        {child_view}::<ValidatedTreeContext>::__from_validated_child_node(\n            node,\n            self.__invocation_states.as_deref(),\n        )\n    }}",
                child.name
            );
            render_required_accessor_validation(
                &mut rendered.validation,
                &method,
                validation_error_name,
            );
        } else {
            let _ = writeln!(
                rendered.recovered,
                "    pub fn {method}(&self) -> Option<{child_view}<'a>> {{\n        __rule_children(self.__node, {child_index})\n            .next()\n            .map(|node| {child_view}::__from_child_node(node, self.__invocation_states.as_deref()))\n    }}"
            );
            let _ = writeln!(
                rendered.validated,
                "    pub fn {method}(&self) -> Option<{child_view}<'a, ValidatedTreeContext>> {{\n        __rule_children(self.__node, {child_index})\n            .next()\n            .map(|node| {child_view}::<ValidatedTreeContext>::__from_validated_child_node(node, self.__invocation_states.as_deref()))\n    }}"
            );
        }
        if antlr4rust_compat && !cardinality.is_repeated() {
            render_antlr4rust_single_rule_accessor(
                &mut rendered.compatibility,
                Antlr4RustSingleRuleAccessorRender {
                    source_name: &child.name,
                    native_method: &method,
                    child_view,
                    required: cardinality.is_required_single(),
                    context_wrapper: antlr4rust_context_wrapper,
                },
                &mut emitted_compatibility_methods,
            );
        }
    }
    for (token_name, token_type) in token_accessors
        .iter()
        .filter(|(token_name, _)| child_cardinalities.contains_key(token_name.as_str()))
    {
        let cardinality = child_cardinalities[token_name.as_str()];
        let stem = accessor_stem(token_name);
        let preferred = if cardinality.is_repeated() {
            format!("{stem}_tokens")
        } else {
            format!("{stem}_token")
        };
        let method = allocate_context_method(
            preferred,
            &format!("{stem}_terminal_child"),
            &mut used_methods,
        );
        if cardinality.is_repeated() {
            let _ = writeln!(
                rendered.recovered,
                "    pub fn {method}(&self) -> impl Iterator<Item = TerminalNode<'a>> + '_ {{\n        __token_children(self.__node, {token_type}).map(TerminalNode::new)\n    }}"
            );
            let _ = writeln!(
                rendered.validated,
                "    pub fn {method}(&self) -> impl Iterator<Item = TerminalNode<'a>> + '_ {{\n        __token_children(self.__node, {token_type}).map(TerminalNode::new)\n    }}"
            );
            render_repeated_accessor_validation(
                &mut rendered.validation,
                &method,
                view_name,
                token_name,
                cardinality.min,
                validation_error_name,
            );
            if antlr4rust_compat {
                render_antlr4rust_indexed_token_accessor(
                    &mut rendered.compatibility,
                    token_name,
                    &method,
                    &mut emitted_compatibility_methods,
                );
                render_antlr4rust_token_all_accessor(
                    &mut rendered.compatibility,
                    token_name,
                    &method,
                    &mut emitted_compatibility_methods,
                );
            }
        } else if cardinality.is_required_single() {
            let _ = writeln!(
                rendered.recovered,
                "    pub fn {method}(&self) -> Result<TerminalNode<'a>, MissingChildError> {{\n        __token_children(self.__node, {token_type})\n            .next()\n            .map(TerminalNode::new)\n            .ok_or_else(|| MissingChildError::new(\"{view_name}\", \"{token_name}\"))\n    }}"
            );
            let _ = writeln!(
                rendered.validated,
                "    pub fn {method}(&self) -> TerminalNode<'a> {{\n        let Some(node) = __token_children(self.__node, {token_type}).next() else {{\n            unreachable!(\"validated {view_name} is missing required child {token_name}\")\n        }};\n        TerminalNode::new(node)\n    }}"
            );
            render_required_accessor_validation(
                &mut rendered.validation,
                &method,
                validation_error_name,
            );
            if antlr4rust_compat {
                render_antlr4rust_single_token_accessor(
                    &mut rendered.compatibility,
                    token_name,
                    &method,
                    true,
                    &mut emitted_compatibility_methods,
                );
            }
        } else {
            let _ = writeln!(
                rendered.recovered,
                "    pub fn {method}(&self) -> Option<TerminalNode<'a>> {{\n        __token_children(self.__node, {token_type})\n            .next()\n            .map(TerminalNode::new)\n    }}"
            );
            let _ = writeln!(
                rendered.validated,
                "    pub fn {method}(&self) -> Option<TerminalNode<'a>> {{\n        __token_children(self.__node, {token_type})\n            .next()\n            .map(TerminalNode::new)\n    }}"
            );
            if antlr4rust_compat {
                render_antlr4rust_single_token_accessor(
                    &mut rendered.compatibility,
                    token_name,
                    &method,
                    false,
                    &mut emitted_compatibility_methods,
                );
            }
        }
    }
    for label in label_accessors {
        let stem = accessor_stem(&label.source_name);
        let method = allocate_context_method(
            rust_function_name(&label.source_name),
            &format!("{stem}_label"),
            &mut used_methods,
        );
        if label.token_types.is_empty()
            && let Some(child_index) = model
                .rules
                .iter()
                .position(|child| child.name == label.target)
        {
            let child_view = &context_names.rules[child_index].context_type;
            render_rule_label_accessor(
                &mut rendered,
                RuleLabelAccessorRender {
                    method: &method,
                    view_name,
                    child_view,
                    child_index,
                    label,
                    validation_error_name,
                },
            );
            continue;
        }
        if label.token_types.is_empty() {
            continue;
        }
        render_token_label_accessor(
            &mut rendered,
            &method,
            view_name,
            label,
            validation_error_name,
        );
    }
    rendered
}
