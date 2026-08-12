// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
fn structural_attr_decl(attribute: &AttributeSymbol) -> embedded::AttrDecl {
    embedded::AttrDecl {
        name: attribute.name.clone(),
        ty: embedded::map_attr_type(&attribute.ty),
    }
}

fn structural_rule_alternatives(rule: &Rule, vocabulary: &Vocabulary) -> Vec<embedded::AltModel> {
    let Some(left_recursion) = &rule.left_recursion else {
        return rule
            .block
            .alternatives
            .iter()
            .map(|alternative| structural_alt_model(alternative, None, vocabulary))
            .collect();
    };

    left_recursion
        .original_to_rewritten
        .iter()
        .filter_map(|(original, rewritten)| {
            let alternative = find_alternative(&rule.block, *rewritten)?;
            let leading_ref = left_recursion
                .alternative_kinds
                .get(original)
                .is_some_and(|kind| {
                    matches!(
                        kind,
                        LeftRecursiveAlternativeKind::Binary | LeftRecursiveAlternativeKind::Suffix
                    )
                })
                .then(|| {
                    let removed = left_recursion
                        .deleted_labels
                        .values()
                        .find(|removed| removed.original_alternative == *original);
                    embedded::ElementRef {
                        label: removed.map(|removed| removed.label.name.clone()),
                        target: removed
                            .map_or_else(|| rule.name.clone(), |removed| removed.target.clone()),
                        token_types: Vec::new(),
                        is_block: false,
                        is_list: removed
                            .is_some_and(|removed| removed.label.kind == LabelKind::List),
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
                    }
                });
            Some(structural_alt_model(alternative, leading_ref, vocabulary))
        })
        .collect()
}

fn find_alternative(block: &Block, id: AlternativeId) -> Option<&Alternative> {
    for alternative in &block.alternatives {
        if alternative.id == id {
            return Some(alternative);
        }
        for element in &alternative.elements {
            if let ElementKind::Block(nested) = &element.kind
                && let Some(found) = find_alternative(nested, id)
            {
                return Some(found);
            }
        }
    }
    None
}

fn structural_alt_model(
    alternative: &Alternative,
    removed_leading_ref: Option<embedded::ElementRef>,
    vocabulary: &Vocabulary,
) -> embedded::AltModel {
    let leading_target = removed_leading_ref
        .as_ref()
        .map(|element| element.target.clone());
    let mut children = structural_context_children(&alternative.elements, vocabulary);
    if let Some(leading) = &removed_leading_ref {
        add_child_cardinality(
            &mut children,
            &leading.target,
            embedded::ChildCardinality::ONE,
        );
    }
    let mut refs = removed_leading_ref.into_iter().collect();
    collect_structural_context_refs(&alternative.elements, &mut refs, true, vocabulary);
    embedded::AltModel {
        label: alternative.label.as_ref().map(|label| label.value.clone()),
        span: (
            usize::try_from(alternative.span.bytes.start).expect("source offset exceeds usize"),
            usize::try_from(alternative.span.bytes.end).expect("source offset exceeds usize"),
        ),
        refs,
        children,
        leading_target: leading_target.or_else(|| {
            alternative
                .elements
                .first()
                .and_then(structural_leading_target)
        }),
    }
}

fn collect_structural_context_refs(
    elements: &[Element],
    refs: &mut Vec<embedded::ElementRef>,
    stable_accessor: bool,
    vocabulary: &Vocabulary,
) {
    collect_structural_context_refs_with_cardinality(
        elements,
        refs,
        StructuralRefContext {
            stable_accessor,
            enclosing_cardinality: embedded::ChildCardinality::ONE,
            branch_local_cardinality: embedded::ChildCardinality::ONE,
            choice_branch: &[],
            choice_arity: &[],
            choice_spans: &[],
            group_spans: &[],
            branch_spans: &[],
        },
        vocabulary,
    );
}

/// Traversal state threaded down while flattening an alternative's elements.
#[derive(Clone, Copy)]
struct StructuralRefContext<'a> {
    stable_accessor: bool,
    /// Cardinality contributed by every enclosing quantifier *and* choice split.
    enclosing_cardinality: embedded::ChildCardinality,
    /// The same, but assuming each enclosing choice took this branch.
    branch_local_cardinality: embedded::ChildCardinality,
    choice_branch: &'a [(usize, usize)],
    choice_arity: &'a [usize],
    choice_spans: &'a [(usize, usize)],
    group_spans: &'a [embedded::GroupSpan],
    branch_spans: &'a [(usize, usize)],
}

fn collect_structural_context_refs_with_cardinality(
    elements: &[Element],
    refs: &mut Vec<embedded::ElementRef>,
    context: StructuralRefContext<'_>,
    vocabulary: &Vocabulary,
) {
    let StructuralRefContext {
        stable_accessor,
        enclosing_cardinality,
        branch_local_cardinality,
        choice_branch,
        choice_arity,
        choice_spans,
        group_spans,
        branch_spans,
    } = context;
    // Tracks whether any terminal-bearing ref has been emitted on this path, so an
    // element can record whether it is the first terminal.
    // Only refs on *this* branch's path count: `refs` is shared across every branch
    // of an enclosing choice, so terminals emitted for a sibling branch must not
    // make this branch's elements look non-leading. `(x=A | x='a')` has each
    // declaration first on its own path.
    let mut seen_terminal = branch_local_cardinality.max.is_none_or(|max| max != 0)
        && refs.iter().any(|existing| {
            !existing.token_types.is_empty()
                && existing.cardinality.max != Some(0)
                && !existing.choice_branch.iter().any(|(choice, branch)| {
                    choice_branch
                        .iter()
                        .any(|(mine, my_branch)| choice == mine && branch != my_branch)
                })
        });
    for element in elements {
        let label = element.label.as_ref().map(|label| label.name.clone());
        let is_list = element
            .label
            .as_ref()
            .is_some_and(|label| label.kind == LabelKind::List);
        let cardinality = multiply_child_cardinalities(
            enclosing_cardinality,
            quantified_cardinality(embedded::ChildCardinality::ONE, element.quantifier),
        );
        // Same product, but against the cardinality this element would have if
        // every enclosing choice took its branch — so a `min: 0` here comes only
        // from a quantifier, never from branch membership.
        let branch_local = multiply_child_cardinalities(
            branch_local_cardinality,
            quantified_cardinality(embedded::ChildCardinality::ONE, element.quantifier),
        );
        match &element.kind {
            ElementKind::RuleCall(call) => refs.push(embedded::ElementRef {
                label,
                target: call.name.clone(),
                token_types: Vec::new(),
                is_block: false,
                is_list,
                cardinality,
                stable_accessor,
                choice_branch: choice_branch.to_vec(),
                choice_arity: choice_arity.to_vec(),
                choice_spans: choice_spans.to_vec(),
                group_spans: group_spans.to_vec(),
                branch_spans: branch_spans.to_vec(),
                leading_terminal: !seen_terminal,
                span: Some((
                    element.span.bytes.start as usize,
                    element.span.bytes.end as usize,
                )),
                branch_local_cardinality: branch_local,
                group_local_cardinality: quantified_cardinality(
                    embedded::ChildCardinality::ONE,
                    element.quantifier,
                ),
            }),
            ElementKind::Terminal(terminal) => {
                refs.push(embedded::ElementRef {
                    label,
                    target: structural_terminal_target(terminal),
                    token_types: structural_terminal_token_types(terminal, vocabulary),
                    is_block: !matches!(terminal, Terminal::Token(_)),
                    is_list,
                    cardinality,
                    stable_accessor,
                    choice_branch: choice_branch.to_vec(),
                    choice_arity: choice_arity.to_vec(),
                    choice_spans: choice_spans.to_vec(),
                    group_spans: group_spans.to_vec(),
                    branch_spans: branch_spans.to_vec(),
                    leading_terminal: !seen_terminal,
                    span: Some((
                        element.span.bytes.start as usize,
                        element.span.bytes.end as usize,
                    )),
                    branch_local_cardinality: branch_local,
                    group_local_cardinality: quantified_cardinality(
                        embedded::ChildCardinality::ONE,
                        element.quantifier,
                    ),
                });
            }
            ElementKind::Block(block) => {
                let token_types = structural_block_token_types(block, vocabulary);
                // A token-only block collapses into a single group ref, which is
                // what makes `x=(A | B)` one labeled token child. That only
                // holds when the label sits *on* the group: when the block is
                // unlabeled and the labels sit inside it (`(x=A)?`), collapsing
                // would swallow them, so descend and let the inner refs carry
                // their own labels.
                // A block holding an action or predicate must also stay expanded even
                // when it is token-only: collapsing it discards the per-branch spans
                // that place the action, so `x=A? (A | B {$x.text})` could not tell
                // that the action runs only where the sibling `A` cannot shadow `x`.
                if !token_types.is_empty()
                    && (label.is_some()
                        || !(structural_block_labels_inside(block)
                            || structural_block_holds_action(block)))
                {
                    refs.push(embedded::ElementRef {
                        label,
                        target: String::new(),
                        token_types,
                        is_block: true,
                        is_list,
                        cardinality,
                        stable_accessor,
                        choice_branch: choice_branch.to_vec(),
                        choice_arity: choice_arity.to_vec(),
                        choice_spans: choice_spans.to_vec(),
                        group_spans: group_spans.to_vec(),
                        branch_spans: branch_spans.to_vec(),
                        leading_terminal: !seen_terminal,
                        span: Some((
                            element.span.bytes.start as usize,
                            element.span.bytes.end as usize,
                        )),
                        branch_local_cardinality: branch_local,
                        group_local_cardinality: quantified_cardinality(
                            embedded::ChildCardinality::ONE,
                            element.quantifier,
                        ),
                    });
                    // The collapsed group *is* a terminal child, so record it before
                    // skipping the rest of the loop body — otherwise the next element
                    // would also be classified as leading.
                    seen_terminal = true;
                    continue;
                }
                if label.is_some() {
                    refs.push(embedded::ElementRef {
                        label,
                        target: String::new(),
                        token_types: Vec::new(),
                        is_block: true,
                        is_list,
                        cardinality,
                        stable_accessor: false,
                        choice_branch: choice_branch.to_vec(),
                        choice_arity: choice_arity.to_vec(),
                        choice_spans: choice_spans.to_vec(),
                        group_spans: group_spans.to_vec(),
                        branch_spans: branch_spans.to_vec(),
                        leading_terminal: !seen_terminal,
                        span: Some((
                            element.span.bytes.start as usize,
                            element.span.bytes.end as usize,
                        )),
                        branch_local_cardinality: branch_local,
                        group_local_cardinality: quantified_cardinality(
                            embedded::ChildCardinality::ONE,
                            element.quantifier,
                        ),
                    });
                }
                // Refs from one alternative of a *choice* are present only when
                // the parse took that branch, and the flattened CST does not
                // record which. Clearing the lower bound states that honestly:
                // a sibling alternative matching the same target then reads as
                // inexact, so the occurrence-lookup guards in
                // `context_label_accessor` reject exactly the layouts where
                // positional access could resolve to another branch's child.
                let branch_cardinality = if block.alternatives.len() > 1 {
                    embedded::ChildCardinality {
                        min: 0,
                        max: cardinality.max,
                    }
                } else {
                    cardinality
                };
                // Where the expanded refs start, so the terminal state can be read
                // back off what the branches actually emitted (below).
                let refs_before_block = refs.len();
                for (branch, alternative) in block.alternatives.iter().enumerate() {
                    // Tag each branch of a *choice* with `(block id, branch)` so
                    // consumers can tell mutually exclusive refs from sequential
                    // ones. A single-alternative block adds no exclusivity, so it
                    // keeps whatever tag it inherited.
                    let mut nested_branch = choice_branch.to_vec();
                    let mut nested_arity = choice_arity.to_vec();
                    let mut nested_spans = choice_spans.to_vec();
                    // Every block counts here, single-alternative groups included.
                    let mut nested_branch_spans = branch_spans.to_vec();
                    let mut nested_groups = group_spans.to_vec();
                    nested_groups.push(embedded::GroupSpan {
                        start: block.span.bytes.start as usize,
                        end: block.span.bytes.end as usize,
                        // Only a quantifier that can yield nothing relaxes the
                        // elements inside.
                        optional: quantified_cardinality(
                            embedded::ChildCardinality::ONE,
                            element.quantifier,
                        )
                        .min == 0,
                        // A star/plus group can run more than once, so even a group
                        // known to have run contributes an unfixed count.
                        repeated: quantified_cardinality(
                            embedded::ChildCardinality::ONE,
                            element.quantifier,
                        )
                        .is_repeated(),
                    });
                    if block.alternatives.len() > 1 {
                        nested_branch.push((block.syntax.index(), branch));
                        nested_arity.push(block.alternatives.len());
                        nested_spans.push((
                            block.span.bytes.start as usize,
                            block.span.bytes.end as usize,
                        ));
                        nested_branch_spans.push((
                            alternative.span.bytes.start as usize,
                            alternative.span.bytes.end as usize,
                        ));
                    }
                    collect_structural_context_refs_with_cardinality(
                        &alternative.elements,
                        refs,
                        StructuralRefContext {
                            stable_accessor,
                            enclosing_cardinality: branch_cardinality,
                            // Inside a branch the group's own quantifier still
                            // applies, but the choice split does not.
                            branch_local_cardinality: branch_local,
                            choice_branch: &nested_branch,
                            choice_arity: &nested_arity,
                            choice_spans: &nested_spans,
                            group_spans: &nested_groups,
                            branch_spans: &nested_branch_spans,
                        },
                        vocabulary,
                    );
                }
                // An expanded block still matched terminals, and the collapsibility
                // helper below cannot see them: `structural_block_token_types`
                // returns empty for any block that is not one-element-per-branch, so
                // `(B y=C? | )` reported no tokens and left the *following* element
                // marked as leading. That false state then let a mixed
                // token/literal merge through in `(B y=C? | ) x=A | x='a'`.
                //
                // Read it back off the refs the branches actually emitted instead.
                // `leading_terminal` is a *claim* that the element is the first
                // terminal — which is what makes a block-positional index and a
                // same-type index agree at 0 — so it has to hold on every path. Any
                // branch that *can* match a terminal falsifies it, hence `any` over
                // `cardinality.max != Some(0)` rather than agreement across branches.
                if refs[refs_before_block..].iter().any(|candidate| {
                    !candidate.token_types.is_empty() && candidate.cardinality.max != Some(0)
                }) {
                    seen_terminal = true;
                }
            }
            ElementKind::Set { inverted, elements } => {
                refs.push(embedded::ElementRef {
                    label,
                    target: String::new(),
                    token_types: structural_set_token_types(*inverted, elements, vocabulary),
                    is_block: true,
                    is_list,
                    cardinality,
                    stable_accessor,
                    choice_branch: choice_branch.to_vec(),
                    choice_arity: choice_arity.to_vec(),
                    choice_spans: choice_spans.to_vec(),
                    group_spans: group_spans.to_vec(),
                    branch_spans: branch_spans.to_vec(),
                    leading_terminal: !seen_terminal,
                    span: Some((
                        element.span.bytes.start as usize,
                        element.span.bytes.end as usize,
                    )),
                    branch_local_cardinality: branch_local,
                    group_local_cardinality: quantified_cardinality(
                        embedded::ChildCardinality::ONE,
                        element.quantifier,
                    ),
                });
            }
            ElementKind::Range(..) if label.is_some() => refs.push(embedded::ElementRef {
                label,
                target: String::new(),
                token_types: Vec::new(),
                is_block: false,
                is_list,
                cardinality,
                stable_accessor: false,
                choice_branch: choice_branch.to_vec(),
                choice_arity: choice_arity.to_vec(),
                choice_spans: choice_spans.to_vec(),
                group_spans: group_spans.to_vec(),
                branch_spans: branch_spans.to_vec(),
                leading_terminal: !seen_terminal,
                span: Some((
                    element.span.bytes.start as usize,
                    element.span.bytes.end as usize,
                )),
                branch_local_cardinality: branch_local,
                group_local_cardinality: quantified_cardinality(
                    embedded::ChildCardinality::ONE,
                    element.quantifier,
                ),
            }),
            ElementKind::Range(..)
            | ElementKind::Action { .. }
            | ElementKind::Predicate { .. }
            | ElementKind::Epsilon => {}
        }
        if !structural_element_token_types(element, vocabulary).is_empty() {
            seen_terminal = true;
        }
    }
}

fn structural_terminal_target(terminal: &Terminal) -> String {
    match terminal {
        Terminal::Token(name) | Terminal::Literal(name) | Terminal::LexerCharSet(name) => {
            name.clone()
        }
        Terminal::Eof => "EOF".to_owned(),
        Terminal::Wildcard => String::new(),
    }
}

fn structural_terminal_token_types(terminal: &Terminal, vocabulary: &Vocabulary) -> Vec<i32> {
    if matches!(terminal, Terminal::Wildcard) {
        return (1..=vocabulary.max_token_type()).collect();
    }
    let token_type = match terminal {
        Terminal::Token(name) => vocabulary.by_name.get(name).copied(),
        Terminal::Literal(literal) => vocabulary.by_literal.get(literal).copied(),
        Terminal::Eof => Some(TOKEN_EOF),
        Terminal::LexerCharSet(_) | Terminal::Wildcard => None,
    };
    token_type.into_iter().collect()
}

pub(crate) fn structural_set_token_types(
    inverted: bool,
    elements: &[SetElement],
    vocabulary: &Vocabulary,
) -> Vec<i32> {
    let mut members = BTreeSet::new();
    for element in elements {
        match element {
            SetElement::Terminal { value, .. } => {
                members.extend(structural_terminal_token_types(value, vocabulary));
            }
            SetElement::Range { start, stop, .. } => {
                let Some(start) = vocabulary.by_literal.get(start).copied() else {
                    continue;
                };
                let Some(stop) = vocabulary.by_literal.get(stop).copied() else {
                    continue;
                };
                if start <= stop {
                    members.extend(start..=stop);
                }
            }
        }
    }
    if inverted {
        (1..=vocabulary.max_token_type())
            .filter(|token_type| !members.contains(token_type))
            .collect()
    } else {
        members.into_iter().collect()
    }
}

fn structural_element_token_types(element: &Element, vocabulary: &Vocabulary) -> Vec<i32> {
    match &element.kind {
        ElementKind::Terminal(terminal) => structural_terminal_token_types(terminal, vocabulary),
        ElementKind::Set { inverted, elements } => {
            structural_set_token_types(*inverted, elements, vocabulary)
        }
        ElementKind::Block(block) => structural_block_token_types(block, vocabulary),
        ElementKind::RuleCall(_)
        | ElementKind::Range(..)
        | ElementKind::Action { .. }
        | ElementKind::Predicate { .. }
        | ElementKind::Epsilon => Vec::new(),
    }
}

fn structural_block_token_types(block: &Block, vocabulary: &Vocabulary) -> Vec<i32> {
    let mut token_types = BTreeSet::new();
    for alternative in &block.alternatives {
        let mut elements = alternative.elements.iter().filter(|element| {
            !matches!(
                element.kind,
                ElementKind::Action { .. } | ElementKind::Predicate { .. } | ElementKind::Epsilon
            )
        });
        let Some(element) = elements.next() else {
            return Vec::new();
        };
        if elements.next().is_some() || element.quantifier != Quantifier::One {
            return Vec::new();
        }
        let alternative_types = structural_element_token_types(element, vocabulary);
        if alternative_types.is_empty() {
            return Vec::new();
        }
        token_types.extend(alternative_types);
    }
    token_types.into_iter().collect()
}

/// Whether `block` contains an action or predicate at any depth. Such a block must
/// not collapse into a single token-group ref: the collapse discards the per-branch
/// spans that decide which branch an action sits in.
fn structural_block_holds_action(block: &Block) -> bool {
    block
        .alternatives
        .iter()
        .flat_map(|alternative| &alternative.elements)
        .any(|element| match &element.kind {
            ElementKind::Action { .. } | ElementKind::Predicate { .. } => true,
            ElementKind::Block(nested) => structural_block_holds_action(nested),
            _ => false,
        })
}

/// Whether any element anywhere inside `block` carries a label, i.e. the block
/// is a grouping wrapper around labeled elements (`(x=A)?`) rather than a
/// labeled token group (`x=(A | B)`).
///
/// The search descends nested blocks: extra grouping levels (`((x=A))?`) are
/// syntactically inert, so a label buried under them must still prevent the
/// collapse that would discard it.
fn structural_block_labels_inside(block: &Block) -> bool {
    block
        .alternatives
        .iter()
        .flat_map(|alternative| &alternative.elements)
        .any(|element| {
            element.label.is_some()
                || matches!(&element.kind, ElementKind::Block(nested)
                    if structural_block_labels_inside(nested))
        })
}

fn structural_terminal_child_target(
    terminal: &Terminal,
    vocabulary: &Vocabulary,
) -> Option<String> {
    match terminal {
        Terminal::Token(name) => Some(name.clone()),
        Terminal::Literal(literal) => {
            let token_type = vocabulary.by_literal.get(literal)?;
            structural_token_child_target(*token_type, vocabulary)
        }
        Terminal::Eof => Some("EOF".to_owned()),
        Terminal::LexerCharSet(_) | Terminal::Wildcard => None,
    }
}

fn structural_token_child_target(token_type: i32, vocabulary: &Vocabulary) -> Option<String> {
    if token_type == TOKEN_EOF {
        return Some("EOF".to_owned());
    }
    vocabulary
        .tokens
        .iter()
        .filter(|token| token.number == token_type)
        .filter_map(|token| token.name.as_ref())
        .find(|name| !name.starts_with("T__"))
        .cloned()
}

fn structural_set_children(
    inverted: bool,
    elements: &[SetElement],
    vocabulary: &Vocabulary,
) -> BTreeMap<String, embedded::ChildCardinality> {
    if inverted {
        return BTreeMap::new();
    }
    let token_types = structural_set_token_types(inverted, elements, vocabulary);
    let cardinality = embedded::ChildCardinality {
        min: usize::from(token_types.len() == 1),
        max: Some(1),
    };
    token_types
        .into_iter()
        .filter_map(|token_type| {
            structural_token_child_target(token_type, vocabulary)
                .map(|target| (target, cardinality))
        })
        .collect()
}

fn structural_context_children(
    elements: &[Element],
    vocabulary: &Vocabulary,
) -> BTreeMap<String, embedded::ChildCardinality> {
    let mut children = BTreeMap::new();
    for element in elements {
        let mut element_children = match &element.kind {
            ElementKind::RuleCall(call) => {
                BTreeMap::from([(call.name.clone(), embedded::ChildCardinality::ONE)])
            }
            ElementKind::Terminal(terminal) => {
                structural_terminal_child_target(terminal, vocabulary)
                    .map(|target| BTreeMap::from([(target, embedded::ChildCardinality::ONE)]))
                    .unwrap_or_default()
            }
            ElementKind::Block(block) => structural_block_children(block, vocabulary),
            ElementKind::Set { inverted, elements } => {
                structural_set_children(*inverted, elements, vocabulary)
            }
            ElementKind::Range(..)
            | ElementKind::Action { .. }
            | ElementKind::Predicate { .. }
            | ElementKind::Epsilon => BTreeMap::new(),
        };
        for cardinality in element_children.values_mut() {
            *cardinality = quantified_cardinality(*cardinality, element.quantifier);
        }
        for (target, cardinality) in element_children {
            add_child_cardinality(&mut children, &target, cardinality);
        }
    }
    children
}

fn structural_block_children(
    block: &Block,
    vocabulary: &Vocabulary,
) -> BTreeMap<String, embedded::ChildCardinality> {
    choice_child_cardinalities(
        block
            .alternatives
            .iter()
            .map(|alternative| structural_context_children(&alternative.elements, vocabulary)),
    )
}

pub(crate) fn choice_child_cardinalities(
    alternatives: impl IntoIterator<Item = BTreeMap<String, embedded::ChildCardinality>>,
) -> BTreeMap<String, embedded::ChildCardinality> {
    let alternatives = alternatives.into_iter().collect::<Vec<_>>();
    let targets = alternatives
        .iter()
        .flat_map(|alternative| alternative.keys().cloned())
        .collect::<BTreeSet<_>>();
    targets
        .into_iter()
        .map(|target| {
            let mut min = usize::MAX;
            let mut max = Some(0_usize);
            for alternative in &alternatives {
                let cardinality = alternative
                    .get(&target)
                    .copied()
                    .unwrap_or(embedded::ChildCardinality::ZERO);
                min = min.min(cardinality.min);
                max = match (max, cardinality.max) {
                    (Some(current), Some(next)) => Some(current.max(next)),
                    _ => None,
                };
            }
            (
                target,
                embedded::ChildCardinality {
                    min: if min == usize::MAX { 0 } else { min },
                    max,
                },
            )
        })
        .collect()
}

fn add_child_cardinality(
    children: &mut BTreeMap<String, embedded::ChildCardinality>,
    target: &str,
    cardinality: embedded::ChildCardinality,
) {
    let total = children
        .entry(target.to_owned())
        .or_insert(embedded::ChildCardinality::ZERO);
    total.min = total.min.saturating_add(cardinality.min);
    total.max = match (total.max, cardinality.max) {
        (Some(current), Some(next)) => Some(current.saturating_add(next)),
        _ => None,
    };
}

fn quantified_cardinality(
    cardinality: embedded::ChildCardinality,
    quantifier: Quantifier,
) -> embedded::ChildCardinality {
    match quantifier {
        Quantifier::One => cardinality,
        Quantifier::Optional { .. } => embedded::ChildCardinality {
            min: 0,
            max: cardinality.max,
        },
        Quantifier::ZeroOrMore { .. } => embedded::ChildCardinality {
            min: 0,
            max: (cardinality.max == Some(0)).then_some(0),
        },
        Quantifier::OneOrMore { .. } => embedded::ChildCardinality {
            min: cardinality.min,
            max: (cardinality.max == Some(0)).then_some(0),
        },
    }
}

const fn multiply_child_cardinalities(
    left: embedded::ChildCardinality,
    right: embedded::ChildCardinality,
) -> embedded::ChildCardinality {
    let max = match (left.max, right.max) {
        (Some(0), _) | (_, Some(0)) => Some(0),
        (Some(left), Some(right)) => Some(left.saturating_mul(right)),
        _ => None,
    };
    embedded::ChildCardinality {
        min: left.min.saturating_mul(right.min),
        max,
    }
}

fn structural_leading_target(element: &Element) -> Option<String> {
    match &element.kind {
        ElementKind::RuleCall(call) => Some(call.name.clone()),
        ElementKind::Terminal(Terminal::Token(name)) => Some(name.clone()),
        _ => None,
    }
}

/// Sets the rendered template for a lexer predicate coordinate, replacing any
/// existing (translated) entry so a per-coordinate override wins over a
/// built-in translation, or appending a new entry for an uncovered coordinate.
pub(crate) fn set_lexer_predicate_template(
    predicates: &mut Vec<((usize, usize), PredicateTemplate)>,
    coordinate: (usize, usize),
    template: PredicateTemplate,
) {
    if let Some(entry) = predicates.iter_mut().find(|(pred, _)| *pred == coordinate) {
        entry.1 = template;
    } else {
        predicates.push((coordinate, template));
    }
}
