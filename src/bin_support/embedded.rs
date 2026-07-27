//! Embedded-action grammar model and `$`-attribute translator.
//!
//! In embedded mode the generator receives a grammar whose actions and
//! predicates are already **real Rust code** — rendered by the conformance
//! harness through `Rust.test.stg`, exactly like every official ANTLR target
//! renders its `.test.stg` — and splices those bodies verbatim into the
//! generated recognizer. The only rewriting applied is ANTLR's own
//! `$attribute` reference translation (the Rust analog of ANTLR's
//! `ActionTranslator`): `$text`, `$ctx`, `$_p`, rule/token/label references,
//! and rule attribute (`args`/`returns`/`locals`) reads and writes.
//!
//! This module consumes the structural grammar model needed for that
//! translation: per-rule attribute declarations, per-alternative element
//! references with labels (for `$label.attr` occurrence resolution), and
//! `@members` bodies split into struct fields, impl items, and module items.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io;

use crate::templates::{matching_action_brace, skip_ascii_whitespace};

/// One `name: type` attribute declared in a rule's `[...]` args clause or
/// `returns [...]` / `locals [...]` clauses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AttrDecl {
    pub(crate) name: String,
    /// Rust type after mapping (Java `int` -> `i32`, `boolean` -> `bool`, …).
    pub(crate) ty: String,
}

/// Number of children with one grammar target that an alternative can emit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ChildCardinality {
    pub(crate) min: usize,
    /// `None` denotes an unbounded maximum.
    pub(crate) max: Option<usize>,
}

impl ChildCardinality {
    pub(crate) const ZERO: Self = Self {
        min: 0,
        max: Some(0),
    };
    pub(crate) const ONE: Self = Self {
        min: 1,
        max: Some(1),
    };

    pub(crate) const fn is_required_single(self) -> bool {
        self.min == 1 && matches!(self.max, Some(1))
    }

    pub(crate) const fn is_repeated(self) -> bool {
        match self.max {
            Some(max) => max > 1,
            None => true,
        }
    }
}

/// Suffix marking a label that was temporarily renamed while resolving a *sibling*
/// declaration of the same label in isolation. Grammar labels are identifiers, so
/// this cannot collide with a real one.
const SIBLING_DECLARATION_SUFFIX: &str = " (sibling declaration)";

/// One enclosing block: its byte extent, and whether its own quantifier relaxes
/// the lower bound of the elements inside it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GroupSpan {
    pub(crate) start: usize,
    pub(crate) end: usize,
    /// `true` for `(…)?` / `(…)*` — the group may contribute nothing.
    pub(crate) optional: bool,
    /// `true` for `(…)*` / `(…)+` — the group may run more than once, so the number
    /// of children it contributes is not fixed even when it is known to have run.
    pub(crate) repeated: bool,
}

/// One element reference inside an alternative: a rule ref, token ref, or a
/// labeled sub-block, in source order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ElementRef {
    pub(crate) label: Option<String>,
    /// Referenced rule or token spelling; empty for token sets and wildcards.
    pub(crate) target: String,
    /// Token types matched by this element. Empty for rule references.
    pub(crate) token_types: Vec<i32>,
    pub(crate) is_block: bool,
    /// `label+=ref` list label.
    pub(crate) is_list: bool,
    /// Cardinality of this element after its direct EBNF suffix.
    pub(crate) cardinality: ChildCardinality,
    /// Whether source-order occurrence lookup is unambiguous for a generated
    /// label accessor. Single-alternative EBNF groups preserve it; choices opt
    /// out because their flattened CST children do not retain the chosen path.
    pub(crate) stable_accessor: bool,
    /// `(choice id, alternative index)` for every enclosing *multi*-alternative
    /// block, outermost first. Two refs that share a choice id but sit in
    /// different alternatives of it are mutually exclusive: no parse contains
    /// both. Empty means the ref is on the rule's own sequential path.
    ///
    /// The whole ancestry is kept, not just the innermost choice: for
    /// `((x=e | f) | e)` the labeled `x` and the trailing `e` are separated by
    /// the *outer* choice, which an innermost-only tag would lose.
    pub(crate) choice_branch: Vec<(usize, usize)>,
    /// Alternative count of each choice named in `choice_branch`, in the same
    /// order. Recorded at collection time because an *empty* alternative emits no
    /// ref at all, so the branch count cannot be recovered from the refs alone —
    /// `(a=A | )` would otherwise look like a one-branch choice that always
    /// yields an `A`.
    pub(crate) choice_arity: Vec<usize>,
    /// Byte span of each enclosing choice *block*, in the same order as
    /// `choice_branch`. An action lies inside a branch only when its offset falls
    /// within the block's span — refs alone cannot tell `(A | xs+=A) {…}` (action
    /// after the group) from `(A x=A {…} | B)` (action inside it), since both put
    /// branch refs on either side of the action.
    pub(crate) choice_spans: Vec<(usize, usize)>,
    /// Cardinality with every *enclosing* quantifier and choice split treated as
    /// satisfied — only this element's own EBNF suffix applies. An action inside
    /// `(A x=A {…})?` runs only when the group matched, so on that path the
    /// preceding `A` is exactly-once even though both `cardinality` and
    /// `branch_local_cardinality` report `0..1` from the group's `?`.
    pub(crate) group_local_cardinality: ChildCardinality,
    /// Byte span and lower-bound-relaxing flag of *every* enclosing block, including
    /// single-alternative groups that `choice_spans` omits. The flag says whether
    /// that group's own quantifier is what made this element optional (`(…)?` or
    /// `(…)*`), so a group known taken can have *its* contribution removed without
    /// disturbing the others: in `((q) x=q {…})?` the outer `?` is satisfied when the
    /// action runs while the inner `(q)` is mandatory and already closed.
    pub(crate) group_spans: Vec<GroupSpan>,
    /// Byte span of each enclosing choice *alternative* (the branch itself), in the
    /// same order as `choice_branch`. Lets an action be attributed to the branch
    /// whose text contains it, including a branch holding only actions or
    /// predicates — such a branch emits no `ElementRef` at all, so ref spans alone
    /// would attribute the action to a neighbouring branch.
    pub(crate) branch_spans: Vec<(usize, usize)>,
    /// Whether no terminal can precede this element on its own parse path. Only
    /// then do a block read (which indexes every terminal child) and a token read
    /// (which indexes only same-type children) provably agree, so this is what
    /// makes a mixed-mode merge sound — occurrence zero alone is not enough, since
    /// a token-mode zero can still sit at a non-zero terminal position.
    pub(crate) leading_terminal: bool,
    /// Byte span of the element in the grammar source, when known. A mid-rule
    /// action executes at *its* source position, so only refs that start before
    /// the action's offset have been matched when its body runs.
    pub(crate) span: Option<(usize, usize)>,
    /// Cardinality this element would have if every enclosing choice took the
    /// branch containing it — i.e. with only the *quantifiers* applied, not the
    /// `min: 0` that `choice_branch` membership imposes.
    ///
    /// `(a=A | b=A) x=A` and `(a=A | b=A)? x=A` give their branch refs the same
    /// `cardinality` (`0..1`), yet the first choice always yields one `A` and the
    /// second may yield none. Only this field separates them.
    pub(crate) branch_local_cardinality: ChildCardinality,
}

impl ElementRef {
    /// Whether `self` and `other` can both appear in one parse. They cannot when
    /// any choice encloses both in *different* alternatives.
    pub(crate) fn can_coexist_with(&self, other: &Self) -> bool {
        !self.choice_branch.iter().any(|(choice, branch)| {
            other
                .choice_branch
                .iter()
                .any(|(other_choice, other_branch)| {
                    choice == other_choice && branch != other_branch
                })
        })
    }

    /// Drops the choices `discard` selects, keeping every parallel choice array
    /// aligned with `choice_branch`.
    ///
    /// `choice_arity`, `choice_spans`, and `branch_spans` are indexed *by position*
    /// in `choice_branch`, so removing an entry must remove the same position from
    /// each. Truncating to the surviving length instead keeps the outermost
    /// entries — which is wrong whenever an *outer* choice is the one dropped:
    /// `((q | q | b) x=q | c)` would then read the outer choice's arity of 2
    /// against the surviving inner choice, mistaking a three-way choice for an
    /// exhaustive two-way one.
    pub(crate) fn retain_choices(&mut self, mut keep: impl FnMut(usize) -> bool) {
        let mask = self
            .choice_branch
            .iter()
            .map(|&(choice, _)| keep(choice))
            .collect::<Vec<_>>();
        fn retain_by_mask<T>(list: &mut Vec<T>, mask: &[bool]) {
            let mut index = 0;
            list.retain(|_| {
                let kept = mask.get(index).copied().unwrap_or(true);
                index += 1;
                kept
            });
        }
        retain_by_mask(&mut self.choice_branch, &mask);
        retain_by_mask(&mut self.choice_arity, &mask);
        retain_by_mask(&mut self.choice_spans, &mask);
        retain_by_mask(&mut self.branch_spans, &mask);
    }
}

/// One top-level alternative of a parser rule.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AltModel {
    /// `# altLabel`, if present.
    pub(crate) label: Option<String>,
    /// Byte span of the alternative inside the grammar source.
    pub(crate) span: (usize, usize),
    pub(crate) refs: Vec<ElementRef>,
    /// Aggregate child cardinality by referenced rule or symbolic token.
    pub(crate) children: BTreeMap<String, ChildCardinality>,
    /// Target of the first syntactic element when it is a bare (possibly
    /// labeled) rule/token reference; `None` for a leading literal, set,
    /// block, or action. ANTLR's left-recursion transformer only treats an
    /// alternative as an operator alternative when the recursion is the
    /// first element, so `'(' e ')'` stays primary even though its first
    /// *reference* is the rule itself.
    pub(crate) leading_target: Option<String>,
}

impl AltModel {
    /// Whether this is a left-recursive operator alternative of `rule_name`.
    pub(crate) fn is_lr_operator(&self, rule_name: &str) -> bool {
        self.leading_target.as_deref() == Some(rule_name)
    }
}

/// Structural model of one compiled parser rule.
#[derive(Clone, Debug, Default)]
pub(crate) struct RuleModel {
    pub(crate) name: String,
    /// Args, returns and locals, flattened (names are unique per rule in the
    /// runtime testsuite corpus).
    pub(crate) attrs: Vec<AttrDecl>,
    /// Names declared specifically by the rule's `locals [...]` clause.
    pub(crate) local_names: Vec<String>,
    /// Names of the attrs that come from the `[...]` args clause, in order —
    /// call sites initialize these positionally (`a[2]`).
    pub(crate) arg_names: Vec<String>,
    pub(crate) init_body: Option<String>,
    pub(crate) after_body: Option<String>,
    pub(crate) alts: Vec<AltModel>,
}

impl RuleModel {
    pub(crate) const fn has_attrs(&self) -> bool {
        !self.attrs.is_empty()
    }

    fn attr(&self, name: &str) -> Option<&AttrDecl> {
        self.attrs.iter().find(|attr| attr.name == name)
    }

    /// The alternative whose span contains `offset`, if any.
    fn alt_at(&self, offset: usize) -> Option<&AltModel> {
        self.alts
            .iter()
            .find(|alt| alt.span.0 <= offset && offset < alt.span.1)
    }
}

/// One member field declared through the target's field-with-initializer
/// members convention (`i: i32 = 0;`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MemberField {
    pub(crate) name: String,
    pub(crate) ty: String,
    pub(crate) init: String,
}

/// `@members` content split by item kind.
#[derive(Clone, Debug, Default)]
pub(crate) struct MembersModel {
    /// Field declarations lowered onto the recognizer struct.
    pub(crate) fields: Vec<MemberField>,
    /// `fn` items spliced into the recognizer's inherent `impl` block.
    pub(crate) impl_items: Vec<String>,
    /// `struct` / `impl` / attribute-prefixed items emitted at module level
    /// (test listeners, custom nodes, …).
    pub(crate) module_items: Vec<String>,
}

/// Full grammar model for embedded translation.
#[derive(Clone, Debug, Default)]
pub(crate) struct EmbeddedModel {
    /// Parser rules keyed by parser rule index (grammar order).
    pub(crate) rules: Vec<RuleModel>,
    pub(crate) parser_members: MembersModel,
}

/// Where an action body executes, which changes how `$text` translates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActionSite {
    /// Mid-rule action: an `action` local minted by
    /// `parser_action_at_current` is in scope.
    Body,
    /// Rule `@after`: runs after the body, before `finish_rule`.
    After,
    /// Rule `@init`: runs at rule entry.
    Init,
}

/// Maps a grammar attribute type (possibly Java-flavored, possibly already
/// Rust from the rendered templates) onto the Rust type the generated attrs
/// struct uses.
pub(crate) fn map_attr_type(raw: &str) -> String {
    let raw = raw.trim();
    if let Some(inner) = raw
        .strip_prefix("List")
        .map(str::trim_start)
        .and_then(|rest| rest.strip_prefix('<'))
        .and_then(|inner| inner.strip_suffix('>'))
        .map(str::trim)
        .filter(|inner| !inner.is_empty())
    {
        return format!("Vec<{}>", map_attr_type(inner));
    }
    match raw {
        "int" | "Integer" => "i32".to_owned(),
        "boolean" => "bool".to_owned(),
        "float" | "double" => "f64".to_owned(),
        other => other.to_owned(),
    }
}

/// Splits `name: type`, tolerating generic types containing `:` (`Vec<T>` has
/// none today, but `::` paths do appear, e.g. `std::string::String`).
fn split_name_colon_type(part: &str) -> Option<(&str, &str)> {
    let colon = part.find(':')?;
    if part[colon..].starts_with("::") {
        return None;
    }
    let name = part[..colon].trim();
    let ty = part[colon + 1..].trim();
    (is_identifier(name) && !ty.is_empty()).then_some((name, ty))
}

fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

/// Splits a members body into field declarations, impl items, and module
/// items.
pub(crate) fn classify_members(body: &str, members: &mut MembersModel) -> io::Result<()> {
    let mut offset = 0;
    let mut pending_attrs = String::new();
    while offset < body.len() {
        offset = skip_ascii_whitespace(body, offset);
        if offset >= body.len() {
            break;
        }
        let rest = &body[offset..];
        if rest.starts_with("//") {
            offset += rest.find('\n').map_or(rest.len(), |nl| nl + 1);
        } else if rest.starts_with('#') {
            // `#[derive(..)]` / `#[allow(..)]` — attaches to the next item.
            let Some(close) = rest.find(']') else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unterminated attribute in @members block",
                ));
            };
            pending_attrs.push_str(&rest[..=close]);
            pending_attrs.push('\n');
            offset += close + 1;
        } else if rest.starts_with("fn ") {
            let item_end = item_end_from(body, offset)?;
            let mut item = std::mem::take(&mut pending_attrs);
            item.push_str(body[offset..item_end].trim());
            members.impl_items.push(item);
            offset = item_end;
        } else if rest.starts_with("struct ")
            || rest.starts_with("impl ")
            || rest.starts_with("use ")
        {
            let item_end = item_end_from(body, offset)?;
            let mut item = std::mem::take(&mut pending_attrs);
            item.push_str(body[offset..item_end].trim());
            members.module_items.push(item);
            offset = item_end;
        } else if let Some(field) = parse_member_field(&body[offset..]) {
            let (field, consumed) = field;
            members.fields.push(field);
            offset += consumed;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unsupported @members item starting at: {}",
                    &rest[..rest.len().min(60)]
                ),
            ));
        }
    }
    Ok(())
}

/// Finds the end of an item: the matching `}` of its first top-level brace
/// block, or the terminating `;` for braceless items (`use x;`).
fn item_end_from(body: &str, offset: usize) -> io::Result<usize> {
    let mut quoted = false;
    let mut escaped = false;
    let mut index = offset;
    while let Some(ch) = body[index..].chars().next() {
        if escaped {
            escaped = false;
            index += ch.len_utf8();
            continue;
        }
        match ch {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '{' if !quoted => {
                let close = matching_action_brace(body, index + 1).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "unterminated brace in @members item",
                    )
                })?;
                return Ok(close + 1);
            }
            ';' if !quoted => return Ok(index + 1),
            _ => {}
        }
        index += ch.len_utf8();
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "unterminated @members item",
    ))
}

/// Parses one `name: type = init;` member-field declaration; returns the
/// field and the number of bytes consumed.
fn parse_member_field(rest: &str) -> Option<(MemberField, usize)> {
    let semicolon = rest.find(';')?;
    let decl = &rest[..semicolon];
    if decl.contains('{') || decl.contains('(') {
        return None;
    }
    let (name_ty, init) = decl.split_once('=')?;
    let (name, ty) = split_name_colon_type(name_ty.trim())?;
    Some((
        MemberField {
            name: name.to_owned(),
            ty: ty.to_owned(),
            init: init.trim().to_owned(),
        },
        semicolon + 1,
    ))
}

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
                candidate.choice_branch.retain(|(choice, _)| {
                    !element
                        .choice_branch
                        .iter()
                        .any(|(taken, _)| taken == choice)
                });
                let keep = candidate.choice_branch.len();
                candidate.choice_arity.truncate(keep);
                candidate.choice_spans.truncate(keep);
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
                candidate.choice_branch.retain(|(choice, _)| {
                    !element
                        .choice_branch
                        .iter()
                        .any(|(taken, _)| taken == choice)
                });
                let keep = candidate.choice_branch.len();
                candidate.choice_arity.truncate(keep);
                candidate.choice_spans.truncate(keep);
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
        let shadowed_when_absent = element_optional_here
            && after.iter().any(|candidate| {
                !declares_label(candidate)
                    && same_target(candidate)
                    && matched_at_action(candidate)
                    // A sibling branch's child cannot slide into the label's slot
                    // *within a parse that bound the label* — the two never coexist.
                    // It is still a hazard when the read may run with the label
                    // unset, which is when the action is not inside the branch that
                    // separates them. That has to be judged per *choice*: a
                    // rule-wide flag would let an action confined to some unrelated
                    // later choice exempt an earlier sibling
                    // (`({false}? x=A | A) (B {$x} | C)`).
                    && (candidate.can_coexist_with(element)
                        || !Self::action_inside_separating_branch(
                            element,
                            candidate,
                            action_branches.as_deref(),
                        ))
            });
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
    while let Some(dollar) = find_dollar(rest) {
        out.push_str(&rest[..dollar]);
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
    }
    out.push_str(rest);
    Ok(out)
}

/// Finds the next `$` that is outside a string literal.
fn find_dollar(text: &str) -> Option<usize> {
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

fn translate_reference(
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
fn text_expression(ctx: &TranslationCtx<'_>) -> String {
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
fn translate_ctx_member(member: &str, ctx: &TranslationCtx<'_>, body: &str) -> io::Result<String> {
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
fn translate_element_read(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grammar::{ScopeDecl, parse_scope_decls};

    fn model(rules: Vec<RuleModel>) -> EmbeddedModel {
        EmbeddedModel {
            rules,
            parser_members: MembersModel::default(),
        }
    }

    fn rule(name: &str) -> RuleModel {
        RuleModel {
            name: name.to_owned(),
            ..RuleModel::default()
        }
    }

    fn tokens(pairs: &[(&str, i32)]) -> BTreeMap<String, i32> {
        pairs
            .iter()
            .map(|(name, ty)| ((*name).to_owned(), *ty))
            .collect()
    }

    #[test]
    fn maps_attribute_types_for_generated_rust() {
        assert_eq!(map_attr_type("int"), "i32");
        assert_eq!(map_attr_type("Integer"), "i32");
        assert_eq!(map_attr_type("boolean"), "bool");
        assert_eq!(map_attr_type("List<Integer>"), "Vec<i32>");
        assert_eq!(map_attr_type("List < List<Integer> >"), "Vec<Vec<i32>>");
        assert_eq!(map_attr_type("std::string::String"), "std::string::String");
    }

    mod upstream_scope_parsing {
        use super::*;

        const CASES: &[(&str, &str)] = &[
            ("", ""),
            (" ", ""),
            ("int i", "i:int"),
            ("int[] i, int j[]", "i:int[], j:int []"),
            ("Map<A,B>[] i, int j[]", "i:Map<A,B>[], j:int []"),
            ("Map<A,List<B>>[] i", "i:Map<A,List<B>>[]"),
            (
                "int i = 34+a[3], int j[] = new int[34]",
                "i:int=34+a[3], j:int []=new int[34]",
            ),
            ("char *[3] foo = {1,2,3}", "foo:char *[3]={1,2,3}"),
            ("String[] headers", "headers:String[]"),
            ("std::vector<std::string> x", "x:std::vector<std::string>"),
            ("i", "i"),
            ("i,j", "i, j"),
            ("i\t,j, k", "i, j, k"),
            ("x: int", "x:int"),
            ("x :int", "x:int"),
            ("x:int", "x:int"),
            ("x:int=3", "x:int=3"),
            (
                "r:Rectangle=Rectangle(fromLength: 6, fromBreadth: 12)",
                "r:Rectangle=Rectangle(fromLength: 6, fromBreadth: 12)",
            ),
            ("p:pointer to int", "p:pointer to int"),
            ("a: array[3] of int", "a:array[3] of int"),
            ("a \t:\tfunc(array[3] of int)", "a:func(array[3] of int)"),
            ("x:int, y:float", "x:int, y:float"),
            (
                "x:T?, f:func(array[3] of int), y:int",
                "x:T?, f:func(array[3] of int), y:int",
            ),
            ("float64 x = 3", "x:float64=3"),
            ("map[string]int x", "x:map[string]int"),
        ];

        #[test]
        fn argument_declarations_match_java() {
            for &(input, expected) in CASES {
                let actual = parse_scope_decls(input)
                    .into_iter()
                    .map(render)
                    .collect::<Vec<_>>()
                    .join(", ");
                assert_eq!(actual, expected, "input {input:?}");
            }
        }

        fn render(declaration: ScopeDecl) -> String {
            let ty = declaration
                .ty
                .map_or_else(String::new, |ty| format!(":{ty}"));
            let initializer = declaration
                .initializer
                .map_or_else(String::new, |initializer| format!("={initializer}"));
            format!("{}{ty}{initializer}", declaration.name)
        }
    }

    #[test]
    fn translates_attr_and_rule_reads() {
        let mut expression = rule("e");
        expression.attrs.push(AttrDecl {
            name: "v".to_owned(),
            ty: "i32".to_owned(),
        });
        let m = model(vec![rule("s"), expression]);
        let toks = tokens(&[("INT", 1)]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 1,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let translated = translate_body("$v = $INT.int;", &ctx).expect("translates");
        assert!(translated.starts_with("__attrs.v = "), "{translated}");
        assert!(
            translated.contains(
                "child_tokens(self.base.parse_tree_storage(), self.base.token_store(), 1)"
            ),
            "{translated}"
        );

        let parent_ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let read = translate_body("writeln!(self.output(), \"{}\", $e.v);", &parent_ctx)
            .expect("translates");
        assert!(read.contains("generated_attrs::<__RuleAttrs1>"), "{read}");
    }

    #[test]
    fn resolves_structural_labels_within_the_owning_alternative() {
        let mut statement = rule("s");
        statement.alts.push(AltModel {
            label: None,
            span: (10, 20),
            refs: vec![
                ElementRef {
                    label: Some("left".to_owned()),
                    target: "e".to_owned(),
                    token_types: Vec::new(),
                    is_block: false,
                    is_list: false,
                    cardinality: ChildCardinality::ONE,
                    stable_accessor: true,
                    choice_branch: Vec::new(),
                    choice_arity: Vec::new(),
                    choice_spans: Vec::new(),
                    group_spans: Vec::new(),
                    branch_spans: Vec::new(),
                    leading_terminal: true,
                    span: None,
                    branch_local_cardinality: ChildCardinality::ONE,
                    group_local_cardinality: ChildCardinality::ONE,
                },
                ElementRef {
                    label: Some("right".to_owned()),
                    target: "e".to_owned(),
                    token_types: Vec::new(),
                    is_block: false,
                    is_list: false,
                    cardinality: ChildCardinality::ONE,
                    stable_accessor: true,
                    choice_branch: Vec::new(),
                    choice_arity: Vec::new(),
                    choice_spans: Vec::new(),
                    group_spans: Vec::new(),
                    branch_spans: Vec::new(),
                    leading_terminal: true,
                    span: None,
                    branch_local_cardinality: ChildCardinality::ONE,
                    group_local_cardinality: ChildCardinality::ONE,
                },
            ],
            children: BTreeMap::from([(
                "e".to_owned(),
                ChildCardinality {
                    min: 2,
                    max: Some(2),
                },
            )]),
            leading_target: Some("e".to_owned()),
        });
        let mut expression = rule("e");
        expression.attrs.push(AttrDecl {
            name: "v".to_owned(),
            ty: "i32".to_owned(),
        });
        let m = model(vec![statement, expression]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: Some(15),
            site: ActionSite::Body,
            token_types: &toks,
        };

        let translated = translate_body("$right.v", &ctx).expect("translates");
        assert!(translated.contains(".nth(1)"), "{translated}");
        assert!(
            translated.contains("generated_attrs::<__RuleAttrs1>"),
            "{translated}"
        );
    }

    /// A label preceded by a same-target ref from a *sibling* choice branch has
    /// no fixed CST position: `r : (e | x=e) {$x...}` builds one `e` child, so
    /// counting the flattened refs would emit `nth(1)` and silently read an
    /// element the parse never produced. Such a label must stay unresolved and
    /// surface as a translation error.
    #[test]
    fn inexact_preceding_refs_leave_labels_unresolved_instead_of_misindexing() {
        let mut statement = rule("s");
        // `r : (e | x=e) {…}`: the two refs are *sequential* here, not branches of
        // one choice — an unlabeled `e` genuinely precedes the label on the same
        // path, which is what leaves its position unfixed. (The mutually exclusive
        // spelling is covered by `sibling_branch_children_do_not_shadow_an_optional_label`.)
        let branch_ref = |label: Option<&str>| ElementRef {
            label: label.map(ToOwned::to_owned),
            target: "e".to_owned(),
            token_types: Vec::new(),
            is_block: false,
            is_list: false,
            cardinality: ChildCardinality {
                min: 0,
                max: Some(1),
            },
            stable_accessor: true,
            choice_branch: Vec::new(),
            choice_arity: Vec::new(),
            choice_spans: Vec::new(),
            group_spans: Vec::new(),
            branch_spans: Vec::new(),
            leading_terminal: true,
            span: None,
            // Optional on its own path, so the count ahead of the label floats.
            branch_local_cardinality: ChildCardinality {
                min: 0,
                max: Some(1),
            },
            group_local_cardinality: ChildCardinality {
                min: 0,
                max: Some(1),
            },
        };
        statement.alts.push(AltModel {
            label: None,
            span: (10, 20),
            refs: vec![branch_ref(None), branch_ref(Some("x"))],
            children: BTreeMap::from([(
                "e".to_owned(),
                ChildCardinality {
                    min: 1,
                    max: Some(1),
                },
            )]),
            leading_target: Some("e".to_owned()),
        });
        let m = model(vec![statement, rule("e")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: Some(15),
            site: ActionSite::Body,
            token_types: &toks,
        };

        let error = translate_body("$x.text", &ctx).expect_err("must not translate");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("cannot translate $x"), "{error}");
    }

    /// An *optional* label with a following same-target child has no fixed
    /// position either: in `r : ({false}? x=A)? A {$x...}` the mandatory `A`
    /// slides into `nth(0)` whenever the optional group is skipped, so the
    /// action would receive a value for an unset label.
    #[test]
    fn optional_labels_shadowed_by_a_following_child_stay_unresolved() {
        let mut statement = rule("s");
        let token_ref = |label: Option<&str>, min| ElementRef {
            label: label.map(ToOwned::to_owned),
            target: "A".to_owned(),
            token_types: vec![1],
            is_block: false,
            is_list: false,
            cardinality: ChildCardinality { min, max: Some(1) },
            stable_accessor: true,
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
        statement.alts.push(AltModel {
            label: None,
            span: (10, 20),
            // `x=A?` then a mandatory `A`.
            refs: vec![token_ref(Some("x"), 0), token_ref(None, 1)],
            children: BTreeMap::new(),
            leading_target: Some("A".to_owned()),
        });
        let m = model(vec![statement]);
        let toks = tokens(&[("A", 1)]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: Some(15),
            site: ActionSite::Body,
            token_types: &toks,
        };

        let error = translate_body("$x.text", &ctx).expect_err("must not translate");
        assert!(error.to_string().contains("cannot translate $x"), "{error}");
    }

    /// Token groups carry no target, so they must not share one occurrence
    /// bucket: an optional disjoint group ahead of a labeled group
    /// (`r : (A | B)? x=(C | D) {$x...}`) must not poison it. Block-label reads
    /// take the last terminal child and never consult the index at all.
    #[test]
    fn disjoint_token_groups_do_not_poison_a_later_block_label() {
        let mut statement = rule("s");
        // `branch_local_cardinality` mirrors `cardinality` here: the optionality
        // comes from the group's own `?`, not from choice membership.
        let group_ref = |label: Option<&str>, token_types: Vec<i32>, min| ElementRef {
            label: label.map(ToOwned::to_owned),
            target: String::new(),
            token_types,
            is_block: true,
            is_list: false,
            cardinality: ChildCardinality { min, max: Some(1) },
            stable_accessor: true,
            choice_branch: Vec::new(),
            choice_arity: Vec::new(),
            choice_spans: Vec::new(),
            group_spans: Vec::new(),
            branch_spans: Vec::new(),
            leading_terminal: true,
            span: None,
            branch_local_cardinality: ChildCardinality { min, max: Some(1) },
            group_local_cardinality: ChildCardinality { min, max: Some(1) },
        };
        statement.alts.push(AltModel {
            label: None,
            span: (10, 20),
            refs: vec![
                group_ref(None, vec![1, 2], 0),
                group_ref(Some("x"), vec![3, 4], 1),
            ],
            children: BTreeMap::new(),
            leading_target: None,
        });
        let m = model(vec![statement]);
        let toks = tokens(&[("A", 1), ("B", 2), ("C", 3), ("D", 4)]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: Some(15),
            site: ActionSite::Body,
            token_types: &toks,
        };

        let translated = translate_body("$x.text", &ctx).expect("translates");
        assert!(translated.contains("terminal_children"), "{translated}");
        assert!(translated.contains(".last()"), "{translated}");
    }

    /// A list label ahead of a same-target single label still contributes
    /// children, so it must go through the occurrence accounting rather than be
    /// skipped: `r : xs+=e name=e` puts one `e` before `name` (exact, countable
    /// → `nth(1)`), while `r : xs+=e+ name=e` puts an unbounded run there and
    /// leaves no fixed position at all.
    #[test]
    fn list_refs_ahead_of_a_single_label_are_counted_then_poison_when_unbounded() {
        let list_ref = |max| ElementRef {
            label: Some("xs".to_owned()),
            target: "e".to_owned(),
            token_types: Vec::new(),
            is_block: false,
            is_list: true,
            cardinality: ChildCardinality { min: 1, max },
            stable_accessor: true,
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
        let single_ref = ElementRef {
            label: Some("name".to_owned()),
            target: "e".to_owned(),
            token_types: Vec::new(),
            is_block: false,
            is_list: false,
            cardinality: ChildCardinality {
                min: 1,
                max: Some(1),
            },
            stable_accessor: true,
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
        let translate = |max| {
            let mut statement = rule("s");
            statement.alts.push(AltModel {
                label: None,
                span: (10, 20),
                refs: vec![list_ref(max), single_ref.clone()],
                children: BTreeMap::new(),
                leading_target: Some("e".to_owned()),
            });
            let m = model(vec![statement, rule("e")]);
            let toks = tokens(&[]);
            let ctx = TranslationCtx {
                model: &m,
                rule_index: 0,
                body_offset: Some(15),
                site: ActionSite::Body,
                token_types: &toks,
            };
            translate_body("$name.text", &ctx).map_err(|error| error.to_string())
        };

        // Exactly one preceding `e`: the position is known.
        let exact = translate(Some(1)).expect("exact list count still resolves");
        assert!(exact.contains(".nth(1)"), "{exact}");

        // Unbounded run of `e` ahead of the label: no fixed index exists.
        let error = translate(None).expect_err("unbounded list must not resolve");
        assert!(error.contains("cannot translate $name"), "{error}");
    }

    /// A list read yields *every* same-target child, so it can only stand for
    /// the label when no same-target element sits outside it. In the `mixed`
    /// shape (`name=e ... errors+=e`) a `$errors` read would fold in `name`'s
    /// child, so the label must not resolve.
    #[test]
    fn list_labels_sharing_a_target_with_another_label_stay_unresolved() {
        let mut statement = rule("s");
        statement.alts.push(AltModel {
            label: None,
            span: (10, 20),
            refs: vec![
                ElementRef {
                    label: Some("name".to_owned()),
                    target: "e".to_owned(),
                    token_types: Vec::new(),
                    is_block: false,
                    is_list: false,
                    cardinality: ChildCardinality {
                        min: 1,
                        max: Some(1),
                    },
                    stable_accessor: true,
                    choice_branch: Vec::new(),
                    choice_arity: Vec::new(),
                    choice_spans: Vec::new(),
                    group_spans: Vec::new(),
                    branch_spans: Vec::new(),
                    leading_terminal: true,
                    span: None,
                    branch_local_cardinality: ChildCardinality::ONE,
                    group_local_cardinality: ChildCardinality::ONE,
                },
                ElementRef {
                    label: Some("errors".to_owned()),
                    target: "e".to_owned(),
                    token_types: Vec::new(),
                    is_block: false,
                    is_list: true,
                    cardinality: ChildCardinality { min: 0, max: None },
                    stable_accessor: true,
                    choice_branch: Vec::new(),
                    choice_arity: Vec::new(),
                    choice_spans: Vec::new(),
                    group_spans: Vec::new(),
                    branch_spans: Vec::new(),
                    leading_terminal: true,
                    span: None,
                    branch_local_cardinality: ChildCardinality::ONE,
                    group_local_cardinality: ChildCardinality::ONE,
                },
            ],
            children: BTreeMap::new(),
            leading_target: Some("e".to_owned()),
        });
        let m = model(vec![statement, rule("e")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: Some(15),
            site: ActionSite::Body,
            token_types: &toks,
        };

        let error = translate_body("$errors", &ctx).expect_err("must not translate");
        assert!(
            error.to_string().contains("cannot translate $errors"),
            "{error}"
        );

        // `name` is still resolvable: it precedes the list, so its own index is
        // fixed at 0 and the list contributes nothing ahead of it.
        let name = translate_body("$name.text", &ctx).expect("translates");
        assert!(name.contains(".nth(0)"), "{name}");
    }

    /// A block label has no target to query, so its read walks the context's
    /// terminal children by *position*: the count of terminals matched ahead of
    /// the block. That is what makes `t=~'x' 'z' {$t.text}` read the token the
    /// label bound rather than the trailing `'z'` the old `last()` picked
    /// (issue #233). Where the count is not fixed, `last()` remains the fallback.
    #[test]
    fn block_labels_read_the_terminal_the_label_bound() {
        let translate = |action_offset| {
            let mut statement = rule("s");
            statement.alts.push(AltModel {
                label: None,
                span: (0, 100),
                refs: vec![
                    ElementRef {
                        label: Some("x".to_owned()),
                        target: String::new(),
                        token_types: vec![1, 2],
                        is_block: true,
                        is_list: false,
                        cardinality: ChildCardinality {
                            min: 1,
                            max: Some(1),
                        },
                        stable_accessor: true,
                        choice_branch: Vec::new(),
                        choice_arity: Vec::new(),
                        choice_spans: Vec::new(),
                        group_spans: Vec::new(),
                        branch_spans: Vec::new(),
                        leading_terminal: true,
                        span: Some((10, 20)),
                        branch_local_cardinality: ChildCardinality::ONE,
                        group_local_cardinality: ChildCardinality::ONE,
                    },
                    ElementRef {
                        label: None,
                        target: "C".to_owned(),
                        token_types: vec![3],
                        is_block: false,
                        is_list: false,
                        cardinality: ChildCardinality {
                            min: 1,
                            max: Some(1),
                        },
                        stable_accessor: true,
                        choice_branch: Vec::new(),
                        choice_arity: Vec::new(),
                        choice_spans: Vec::new(),
                        group_spans: Vec::new(),
                        branch_spans: Vec::new(),
                        leading_terminal: true,
                        span: Some((30, 31)),
                        branch_local_cardinality: ChildCardinality::ONE,
                        group_local_cardinality: ChildCardinality::ONE,
                    },
                ],
                children: BTreeMap::new(),
                leading_target: None,
            });
            let m = model(vec![statement]);
            let toks = tokens(&[("A", 1), ("B", 2), ("C", 3)]);
            let ctx = TranslationCtx {
                model: &m,
                rule_index: 0,
                body_offset: Some(action_offset),
                site: ActionSite::Body,
                token_types: &toks,
            };
            translate_body("$x.text", &ctx).map_err(|error| error.to_string())
        };

        // The block is the first terminal either way, so the read is `nth(0)` and
        // the trailing `C` cannot be mistaken for it — regardless of whether the
        // action precedes or follows `C`.
        for offset in [25, 40] {
            let translated = translate(offset).expect("a fixed terminal position resolves");
            assert!(translated.contains("terminal_children"), "{translated}");
            assert!(
                translated.contains(".nth(0)"),
                "offset {offset}: {translated}"
            );
        }
    }

    /// A mid-rule action executes at its own source position, so refs written
    /// after it have not been matched and cannot affect its read. Spans decide
    /// this: `r : xs+=A {$xs} A;` iterates the sole child available at the action,
    /// while the same refs read from `@after` see both and must decline.
    #[test]
    fn future_children_do_not_constrain_a_mid_rule_read() {
        let token_ref = |label: Option<&str>, is_list, span| ElementRef {
            label: label.map(ToOwned::to_owned),
            target: "A".to_owned(),
            token_types: vec![1],
            is_block: false,
            is_list,
            cardinality: ChildCardinality {
                min: 1,
                max: Some(1),
            },
            stable_accessor: true,
            choice_branch: Vec::new(),
            choice_arity: Vec::new(),
            choice_spans: Vec::new(),
            group_spans: Vec::new(),
            branch_spans: Vec::new(),
            leading_terminal: true,
            span: Some(span),
            branch_local_cardinality: ChildCardinality::ONE,
            group_local_cardinality: ChildCardinality::ONE,
        };
        let mut statement = rule("s");
        statement.alts.push(AltModel {
            label: None,
            span: (0, 100),
            // `xs+=A {action at 20} A`
            refs: vec![
                token_ref(Some("xs"), true, (10, 11)),
                token_ref(None, false, (30, 31)),
            ],
            children: BTreeMap::new(),
            leading_target: None,
        });
        let m = model(vec![statement]);
        let toks = tokens(&[("A", 1)]);
        let mid_rule = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: Some(20),
            site: ActionSite::Body,
            token_types: &toks,
        };
        let translated = translate_body("$xs", &mid_rule).expect("trailing A has not matched yet");
        assert!(translated.contains("child_tokens"), "{translated}");

        // Read from `@after`, the trailing `A` has matched and would be iterated.
        let after = TranslationCtx {
            body_offset: None,
            site: ActionSite::After,
            ..mid_rule
        };
        let error = translate_body("$xs", &after).expect_err("both children are present by then");
        assert!(
            error.to_string().contains("cannot translate $xs"),
            "{error}"
        );
    }

    /// Several declarations of one label can share a single read when each lowers
    /// to the same query and they are mutually exclusive: `(x=A e {$x} | x=A f
    /// {$x})` resolves, while `x=A | x='a'` resolves too because token-backed
    /// equivalence is by *type*, not by source form or block-ness.
    #[test]
    fn compatible_declarations_share_one_read() {
        // Each declaration is mandatory *within its branch* — `min: 0` on
        // `cardinality` would say the label is genuinely optional, which is a
        // different (and displaceable) shape.
        let decl = |branch, is_block, span| ElementRef {
            label: Some("x".to_owned()),
            target: if is_block { "'a'" } else { "A" }.to_owned(),
            token_types: vec![1],
            is_block,
            is_list: false,
            cardinality: ChildCardinality {
                min: 1,
                max: Some(1),
            },
            stable_accessor: true,
            choice_branch: vec![(5, branch)],
            choice_arity: Vec::new(),
            choice_spans: Vec::new(),
            group_spans: Vec::new(),
            branch_spans: Vec::new(),
            leading_terminal: true,
            span: Some(span),
            branch_local_cardinality: ChildCardinality::ONE,
            group_local_cardinality: ChildCardinality::ONE,
        };
        // `separate_alts` models `x=… | x=…` written as *top-level* alternatives,
        // which the collector emits as two `AltModel`s — the shape a real grammar
        // produces. Both in one `AltModel` instead models a nested `(… | …)` choice.
        let translate = |second: ElementRef, offset, separate_alts: bool| {
            let mut statement = rule("s");
            if separate_alts {
                for (index, declaration) in
                    [decl(0, false, (10, 11)), second].into_iter().enumerate()
                {
                    statement.alts.push(AltModel {
                        label: None,
                        span: (index * 50, index * 50 + 50),
                        refs: vec![declaration],
                        children: BTreeMap::new(),
                        leading_target: None,
                    });
                }
            } else {
                statement.alts.push(AltModel {
                    label: None,
                    span: (0, 100),
                    refs: vec![decl(0, false, (10, 11)), second],
                    children: BTreeMap::new(),
                    leading_target: None,
                });
            }
            let m = model(vec![statement]);
            let toks = tokens(&[("A", 1)]);
            let ctx = TranslationCtx {
                model: &m,
                rule_index: 0,
                body_offset: offset,
                site: if offset.is_some() {
                    ActionSite::Body
                } else {
                    ActionSite::After
                },
                token_types: &toks,
            };
            translate_body("$x.text", &ctx).map_err(|error| error.to_string())
        };

        // `(x=A e {action} | x=A f {action})`: same query, exclusive branches of one
        // nested choice.
        let translated = translate(decl(1, false, (30, 31)), Some(20), false)
            .expect("identical reads in exclusive branches share one lookup");
        assert!(translated.contains(".nth(0)"), "{translated}");

        // `r @after {…} : x=A | x='a';` — literal and symbolic forms of one token
        // type, as *top-level* alternatives. Both resolve at occurrence zero, the
        // index where the block and token coordinate systems coincide.
        let aliased = translate(decl(1, true, (60, 63)), None, true)
            .expect("token-type equivalence ignores source form");
        assert!(aliased.contains(".nth(0)"), "{aliased}");
    }

    /// A *literal* terminal label (`x='b'`) is `is_block` too, so it must take the
    /// same positional terminal count as a labeled group — keying the count on an
    /// empty target instead would leave it counting same-target children while its
    /// read walks every terminal. This is ANTLR's
    /// `ParserErrors/ConjuringUpToken` shape, where `'a'` precedes the label.
    #[test]
    fn literal_terminal_labels_count_every_preceding_terminal() {
        let terminal = |label: Option<&str>, target: &str, token_type, span| ElementRef {
            label: label.map(ToOwned::to_owned),
            target: target.to_owned(),
            token_types: vec![token_type],
            // Literals and groups alike route through the block read.
            is_block: true,
            is_list: false,
            cardinality: ChildCardinality {
                min: 1,
                max: Some(1),
            },
            stable_accessor: true,
            choice_branch: Vec::new(),
            choice_arity: Vec::new(),
            choice_spans: Vec::new(),
            group_spans: Vec::new(),
            branch_spans: Vec::new(),
            leading_terminal: true,
            span: Some(span),
            branch_local_cardinality: ChildCardinality::ONE,
            group_local_cardinality: ChildCardinality::ONE,
        };
        let mut statement = rule("s");
        statement.alts.push(AltModel {
            label: None,
            span: (0, 100),
            // `'a' x='b' {action} 'c'`
            refs: vec![
                terminal(None, "'a'", 1, (10, 13)),
                terminal(Some("x"), "'b'", 2, (14, 17)),
                terminal(None, "'c'", 3, (40, 43)),
            ],
            children: BTreeMap::new(),
            leading_target: None,
        });
        let m = model(vec![statement]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: Some(20),
            site: ActionSite::Body,
            token_types: &toks,
        };

        let translated = translate_body("$x", &ctx).expect("translates");
        assert!(translated.contains("terminal_children"), "{translated}");
        // `'a'` is terminal 0, so the label is terminal 1 — not `last()`, which
        // would become `'c'` once that matched.
        assert!(translated.contains(".nth(1)"), "{translated}");
    }

    /// An alternative that does not declare the label leaves it unset, so the
    /// read has to come up empty there. `r : x=A | A` breaks that: the second
    /// alternative builds an `A` the read would select, reporting a value for a
    /// label the parse never bound. Conversely `r : x=A | B` is fine.
    #[test]
    fn unscoped_reads_reject_alternatives_that_would_satisfy_them_unbound() {
        let token_ref = |label: Option<&str>, target: &str, token_type| ElementRef {
            label: label.map(ToOwned::to_owned),
            target: target.to_owned(),
            token_types: vec![token_type],
            is_block: false,
            is_list: false,
            cardinality: ChildCardinality {
                min: 1,
                max: Some(1),
            },
            stable_accessor: true,
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
        let translate = |second: ElementRef| {
            let mut statement = rule("s");
            for (index, refs) in [vec![token_ref(Some("x"), "A", 1)], vec![second]]
                .into_iter()
                .enumerate()
            {
                statement.alts.push(AltModel {
                    label: None,
                    span: (index * 10, index * 10 + 10),
                    refs,
                    children: BTreeMap::new(),
                    leading_target: None,
                });
            }
            let m = model(vec![statement]);
            let toks = tokens(&[("A", 1), ("B", 2)]);
            let ctx = TranslationCtx {
                model: &m,
                rule_index: 0,
                body_offset: None,
                site: ActionSite::After,
                token_types: &toks,
            };
            translate_body("$x.text", &ctx).map_err(|error| error.to_string())
        };

        // `x=A | A`: the unlabeled `A` satisfies the read with `x` unset.
        let error = translate(token_ref(None, "A", 1))
            .expect_err("an unbound label must not read another alternative's child");
        assert!(error.contains("cannot translate $x"), "{error}");

        // `x=A | B`: nothing in the second alternative can be mistaken for `x`.
        let translated =
            translate(token_ref(None, "B", 2)).expect("disjoint alternative keeps the read");
        assert!(translated.contains(".nth(0)"), "{translated}");
    }

    /// A list read iterates one target, so every declaration of the label must
    /// name that target: `xs+=A xs+=B` would iterate `A` and drop every `B`.
    /// Equivalent *block* labels across alternatives (`x=(A | B) | x=(C | D)`)
    /// conversely stay resolvable, because the block read ignores token sets.
    #[test]
    fn list_declarations_must_share_a_target_while_block_reads_ignore_token_sets() {
        let list_ref = |target: &str, token_type| ElementRef {
            label: Some("xs".to_owned()),
            target: target.to_owned(),
            token_types: vec![token_type],
            is_block: false,
            is_list: true,
            cardinality: ChildCardinality {
                min: 1,
                max: Some(1),
            },
            stable_accessor: true,
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
        let mut statement = rule("s");
        statement.alts.push(AltModel {
            label: None,
            span: (10, 20),
            refs: vec![list_ref("A", 1), list_ref("B", 2)],
            children: BTreeMap::new(),
            leading_target: None,
        });
        let m = model(vec![statement]);
        let toks = tokens(&[("A", 1), ("B", 2)]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: Some(15),
            site: ActionSite::Body,
            token_types: &toks,
        };
        let error = translate_body("$xs", &ctx).expect_err("mixed list targets must not resolve");
        assert!(
            error.to_string().contains("cannot translate $xs"),
            "{error}"
        );

        // Two block labels over different token sets lower to the same read.
        let block_ref = |token_types: Vec<i32>| ElementRef {
            label: Some("x".to_owned()),
            target: String::new(),
            token_types,
            is_block: true,
            is_list: false,
            cardinality: ChildCardinality {
                min: 1,
                max: Some(1),
            },
            stable_accessor: true,
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
        let mut choice = rule("s");
        for (index, refs) in [vec![block_ref(vec![1, 2])], vec![block_ref(vec![3, 4])]]
            .into_iter()
            .enumerate()
        {
            choice.alts.push(AltModel {
                label: None,
                span: (index * 10, index * 10 + 10),
                refs,
                children: BTreeMap::new(),
                leading_target: None,
            });
        }
        let m = model(vec![choice]);
        let toks = tokens(&[("A", 1), ("B", 2), ("C", 3), ("D", 4)]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::After,
            token_types: &toks,
        };
        let translated = translate_body("$x.text", &ctx).expect("equivalent block reads resolve");
        assert!(translated.contains("terminal_children"), "{translated}");
    }

    /// Reads that `translate_element_read` cannot express must stay unresolved
    /// rather than fall through to a different read: a list over a *token group*
    /// (`xs+=(A | B)`) has no target to iterate and would emit
    /// `.last()…collect()` — Rust that does not compile — and a *repeated* single
    /// label (`(x=A)+`) is overwritten each iteration, so a fixed `nth(0)` pins
    /// the first match where ANTLR exposes the latest.
    #[test]
    fn reads_the_translator_cannot_express_stay_unresolved() {
        let translate = |element: ElementRef, read: &str| {
            let mut statement = rule("s");
            statement.alts.push(AltModel {
                label: None,
                span: (10, 20),
                refs: vec![element],
                children: BTreeMap::new(),
                leading_target: None,
            });
            let m = model(vec![statement]);
            let toks = tokens(&[("A", 1), ("B", 2)]);
            let ctx = TranslationCtx {
                model: &m,
                rule_index: 0,
                body_offset: Some(15),
                site: ActionSite::Body,
                token_types: &toks,
            };
            translate_body(read, &ctx).map_err(|error| error.to_string())
        };

        let group_list = ElementRef {
            label: Some("xs".to_owned()),
            target: String::new(),
            token_types: vec![1, 2],
            is_block: true,
            is_list: true,
            cardinality: ChildCardinality { min: 1, max: None },
            stable_accessor: true,
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
        let error = translate(group_list, "$xs").expect_err("no target to iterate");
        assert!(error.contains("cannot translate $xs"), "{error}");

        let repeated_single = ElementRef {
            label: Some("x".to_owned()),
            target: "A".to_owned(),
            token_types: vec![1],
            is_block: false,
            is_list: false,
            cardinality: ChildCardinality { min: 1, max: None },
            stable_accessor: true,
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
        let error = translate(repeated_single, "$x.text").expect_err("no last-occurrence read");
        assert!(error.contains("cannot translate $x"), "{error}");
    }

    /// Mutual exclusion needs the *whole* choice ancestry, not just the innermost
    /// choice. In `((x=e | f) | e)` the label and the trailing `e` are separated
    /// by the outer choice; keeping only the inner tag would make them look
    /// independent and reject a valid action.
    #[test]
    fn nested_choices_keep_their_outer_branch_ancestry() {
        let rule_ref = |label: Option<&str>, branches: Vec<(usize, usize)>, span| ElementRef {
            label: label.map(ToOwned::to_owned),
            target: "e".to_owned(),
            token_types: Vec::new(),
            is_block: false,
            is_list: false,
            cardinality: ChildCardinality {
                min: 0,
                max: Some(1),
            },
            stable_accessor: true,
            choice_branch: branches,
            choice_arity: Vec::new(),
            choice_spans: Vec::new(),
            group_spans: Vec::new(),
            branch_spans: Vec::new(),
            leading_terminal: true,
            span: Some(span),
            branch_local_cardinality: ChildCardinality::ONE,
            group_local_cardinality: ChildCardinality::ONE,
        };
        let mut statement = rule("s");
        statement.alts.push(AltModel {
            label: None,
            span: (0, 100),
            refs: vec![
                // Outer choice 1 branch 0, then inner choice 2 branch 0.
                rule_ref(Some("x"), vec![(1, 0), (2, 0)], (10, 11)),
                // Outer choice 1 branch 1 — excluded by the *outer* choice alone.
                rule_ref(None, vec![(1, 1)], (30, 31)),
            ],
            children: BTreeMap::new(),
            leading_target: Some("e".to_owned()),
        });
        let m = model(vec![statement, rule("e")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: Some(15),
            site: ActionSite::Body,
            token_types: &toks,
        };

        let translated = translate_body("$x.text", &ctx).expect("outer exclusion still applies");
        assert!(translated.contains(".nth(0)"), "{translated}");
    }

    /// Sibling exclusion is valid only for a *mid-rule* action, which is confined
    /// to the branch it is written in. An `@after` body runs whichever branch the
    /// parse took, so `r @after {$x.text} : (x=A | A) EOF;` must decline — the
    /// unlabeled branch's `A` is present when the read executes.
    #[test]
    fn unscoped_bodies_treat_sibling_matches_as_hazards() {
        let branch_ref = |label: Option<&str>, branch, span| ElementRef {
            label: label.map(ToOwned::to_owned),
            target: "A".to_owned(),
            token_types: vec![1],
            is_block: false,
            is_list: false,
            cardinality: ChildCardinality {
                min: 0,
                max: Some(1),
            },
            stable_accessor: true,
            choice_branch: vec![(3, branch)],
            choice_arity: Vec::new(),
            choice_spans: Vec::new(),
            group_spans: Vec::new(),
            branch_spans: Vec::new(),
            leading_terminal: true,
            span: Some(span),
            branch_local_cardinality: ChildCardinality::ONE,
            group_local_cardinality: ChildCardinality::ONE,
        };
        let mut statement = rule("s");
        statement.alts.push(AltModel {
            label: None,
            span: (0, 100),
            refs: vec![
                branch_ref(Some("x"), 0, (10, 11)),
                branch_ref(None, 1, (20, 21)),
            ],
            children: BTreeMap::new(),
            leading_target: Some("A".to_owned()),
        });
        let m = model(vec![statement]);
        let toks = tokens(&[("A", 1)]);
        let after = TranslationCtx {
            model: &m,
            rule_index: 0,
            // `@after`: unscoped, so any branch may have run.
            body_offset: None,
            site: ActionSite::After,
            token_types: &toks,
        };
        let error = translate_body("$x.text", &after).expect_err("sibling match is a hazard here");
        assert!(error.to_string().contains("cannot translate $x"), "{error}");

        // The same refs read from a mid-rule action inside the labeled branch do
        // resolve, since that action cannot run on the sibling branch.
        let body = TranslationCtx {
            body_offset: Some(15),
            site: ActionSite::Body,
            ..after
        };
        let translated =
            translate_body("$x.text", &body).expect("mid-rule action excludes sibling");
        assert!(translated.contains(".nth(0)"), "{translated}");
    }

    /// A token label's read queries by token *type*, so a differently-spelled
    /// terminal with the same type is the same child: with `A : 'a';`,
    /// `r : (xs+=A)? 'a' {$xs}` must decline because the mandatory `'a'` would be
    /// iterated as an `xs` element.
    #[test]
    fn token_labels_compare_types_not_source_spelling() {
        let mut statement = rule("s");
        statement.alts.push(AltModel {
            label: None,
            span: (10, 20),
            refs: vec![
                ElementRef {
                    label: Some("xs".to_owned()),
                    target: "A".to_owned(),
                    token_types: vec![1],
                    is_block: false,
                    is_list: true,
                    cardinality: ChildCardinality {
                        min: 0,
                        max: Some(1),
                    },
                    stable_accessor: true,
                    choice_branch: Vec::new(),
                    choice_arity: Vec::new(),
                    choice_spans: Vec::new(),
                    group_spans: Vec::new(),
                    branch_spans: Vec::new(),
                    leading_terminal: true,
                    span: None,
                    branch_local_cardinality: ChildCardinality::ONE,
                    group_local_cardinality: ChildCardinality::ONE,
                },
                ElementRef {
                    label: None,
                    // Literal spelling differs; the token type is identical.
                    target: "'a'".to_owned(),
                    token_types: vec![1],
                    is_block: false,
                    is_list: false,
                    cardinality: ChildCardinality {
                        min: 1,
                        max: Some(1),
                    },
                    stable_accessor: true,
                    choice_branch: Vec::new(),
                    choice_arity: Vec::new(),
                    choice_spans: Vec::new(),
                    group_spans: Vec::new(),
                    branch_spans: Vec::new(),
                    leading_terminal: true,
                    span: None,
                    branch_local_cardinality: ChildCardinality::ONE,
                    group_local_cardinality: ChildCardinality::ONE,
                },
            ],
            children: BTreeMap::new(),
            leading_target: None,
        });
        let m = model(vec![statement]);
        let toks = tokens(&[("A", 1)]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: Some(15),
            site: ActionSite::Body,
            token_types: &toks,
        };

        let error = translate_body("$xs", &ctx).expect_err("aliased terminal is the same child");
        assert!(
            error.to_string().contains("cannot translate $xs"),
            "{error}"
        );
    }

    /// A same-target ref in a *sibling* choice branch never coexists with the
    /// label, so it cannot displace an optional one: `r : (x=A {$x.text} B | A C)`
    /// must still translate. Only a certainly-matched following child shadows.
    #[test]
    fn sibling_branch_children_do_not_shadow_an_optional_label() {
        let mut statement = rule("s");
        // Two branches of one choice: same choice id, different branch index.
        let branch_ref = |label: Option<&str>, branch, span| ElementRef {
            label: label.map(ToOwned::to_owned),
            target: "A".to_owned(),
            token_types: vec![1],
            is_block: false,
            is_list: false,
            // Mutually exclusive branches both report `min: 0`.
            cardinality: ChildCardinality {
                min: 0,
                max: Some(1),
            },
            stable_accessor: true,
            choice_branch: vec![(7, branch)],
            choice_arity: Vec::new(),
            choice_spans: Vec::new(),
            group_spans: Vec::new(),
            branch_spans: Vec::new(),
            leading_terminal: true,
            span: Some(span),
            branch_local_cardinality: ChildCardinality::ONE,
            group_local_cardinality: ChildCardinality::ONE,
        };
        statement.alts.push(AltModel {
            label: None,
            span: (0, 100),
            // `(x=A {action} B | A C)`: the action sits inside branch 0.
            refs: vec![
                branch_ref(Some("x"), 0, (10, 11)),
                branch_ref(None, 1, (30, 31)),
            ],
            children: BTreeMap::new(),
            leading_target: Some("A".to_owned()),
        });
        let m = model(vec![statement]);
        let toks = tokens(&[("A", 1)]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: Some(15),
            site: ActionSite::Body,
            token_types: &toks,
        };

        let translated = translate_body("$x.text", &ctx).expect("translates");
        assert!(translated.contains(".nth(0)"), "{translated}");

        // The sequential counterpart — `(pred x=A)? A?`, both on the rule's own
        // path — *is* a hazard: the follower may consume the only token while the
        // label is unset. Cardinality alone cannot tell these two apart, which is
        // what `choice_branch` exists for.
        let mut sequential = rule("s");
        let sequential_ref = |label: Option<&str>, span| ElementRef {
            label: label.map(ToOwned::to_owned),
            target: "A".to_owned(),
            token_types: vec![1],
            is_block: false,
            is_list: false,
            cardinality: ChildCardinality {
                min: 0,
                max: Some(1),
            },
            stable_accessor: true,
            choice_branch: Vec::new(),
            choice_arity: Vec::new(),
            choice_spans: Vec::new(),
            group_spans: Vec::new(),
            branch_spans: Vec::new(),
            leading_terminal: true,
            span: Some(span),
            branch_local_cardinality: ChildCardinality::ONE,
            group_local_cardinality: ChildCardinality::ONE,
        };
        sequential.alts.push(AltModel {
            label: None,
            span: (0, 100),
            // `(pred x=A)? A?` with the action last: both have matched by then.
            refs: vec![
                sequential_ref(Some("x"), (10, 11)),
                sequential_ref(None, (12, 13)),
            ],
            children: BTreeMap::new(),
            leading_target: Some("A".to_owned()),
        });
        let m = model(vec![sequential]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: Some(15),
            site: ActionSite::Body,
            token_types: &toks,
        };
        let error = translate_body("$x.text", &ctx).expect_err("sequential follower shadows");
        assert!(error.to_string().contains("cannot translate $x"), "{error}");
    }

    /// A list label repeated within one alternative is the ordinary
    /// comma-separated idiom (`xs+=e (op xs+=e)+`) — every declaration feeds the
    /// same iteration, so repeats must not be mistaken for a conflict. This is
    /// the shape of ANTLR's `ParserExec/ListLabelsOnRuleRefStartOfAlt`
    /// descriptor, read from `@after` across alternatives that declare it plus
    /// one that does not.
    #[test]
    fn repeated_list_declarations_across_alternatives_still_resolve() {
        let list_ref = || ElementRef {
            label: Some("args".to_owned()),
            target: "e".to_owned(),
            token_types: Vec::new(),
            is_block: false,
            is_list: true,
            cardinality: ChildCardinality { min: 1, max: None },
            stable_accessor: true,
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
        let token_ref = ElementRef {
            label: None,
            target: "ID".to_owned(),
            token_types: vec![1],
            is_block: false,
            is_list: false,
            cardinality: ChildCardinality {
                min: 1,
                max: Some(1),
            },
            stable_accessor: true,
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
        let mut statement = rule("s");
        for (index, refs) in [
            // `args+=e (AND args+=e)+` — two declarations, one iteration.
            vec![list_ref(), list_ref()],
            // An alternative that never mentions the label at all.
            vec![token_ref],
        ]
        .into_iter()
        .enumerate()
        {
            statement.alts.push(AltModel {
                label: None,
                span: (index * 10, index * 10 + 10),
                refs,
                children: BTreeMap::new(),
                leading_target: None,
            });
        }
        let m = model(vec![statement, rule("e")]);
        let toks = tokens(&[("ID", 1)]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::After,
            token_types: &toks,
        };

        let translated = translate_body("$args", &ctx).expect("list label resolves");
        assert!(translated.contains("child_rule_trees"), "{translated}");
    }

    /// `@after` / `@init` bodies are not scoped to an alternative, so one read
    /// has to serve whichever alternative the parse took. `r : x=A | x=B` with an
    /// `@after` read of `$x` would emit the `A` lookup and yield a default on the
    /// `B` branch, while `r : x=A B | x=A C` resolves identically in both.
    #[test]
    fn unscoped_bodies_reject_labels_that_resolve_differently_per_alternative() {
        let token_ref = |label: Option<&str>, target: &str, token_type| ElementRef {
            label: label.map(ToOwned::to_owned),
            target: target.to_owned(),
            token_types: vec![token_type],
            is_block: false,
            is_list: false,
            cardinality: ChildCardinality {
                min: 1,
                max: Some(1),
            },
            stable_accessor: true,
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
        let translate = |second: Vec<ElementRef>| {
            let mut statement = rule("s");
            for (index, refs) in [vec![token_ref(Some("x"), "A", 1)], second]
                .into_iter()
                .enumerate()
            {
                statement.alts.push(AltModel {
                    label: None,
                    span: (index * 10, index * 10 + 10),
                    refs,
                    children: BTreeMap::new(),
                    leading_target: None,
                });
            }
            let m = model(vec![statement]);
            let toks = tokens(&[("A", 1), ("B", 2), ("C", 3)]);
            let ctx = TranslationCtx {
                model: &m,
                rule_index: 0,
                // `@after`: no offset, so no single owning alternative.
                body_offset: None,
                site: ActionSite::After,
                token_types: &toks,
            };
            translate_body("$x.text", &ctx).map_err(|error| error.to_string())
        };

        // `x=A | x=B`: the two alternatives need different token lookups.
        let error = translate(vec![token_ref(Some("x"), "B", 2)])
            .expect_err("conflicting per-alternative reads must not translate");
        assert!(error.contains("cannot translate $x"), "{error}");

        // `x=A B | x=A C`: both resolve to the same `A` lookup at occurrence 0.
        let agreed = translate(vec![token_ref(Some("x"), "A", 1), token_ref(None, "C", 3)])
            .expect("agreeing per-alternative reads still translate");
        assert!(agreed.contains(".nth(0)"), "{agreed}");
    }

    /// One label declared over disjoint targets (`r : (x=A | x=B)`) cannot be
    /// served by a single positional read: picking the first declaration yields
    /// an empty value whenever the parse took the other branch.
    #[test]
    fn labels_repeated_over_disjoint_targets_stay_unresolved() {
        let mut statement = rule("s");
        let token_ref = |target: &str, token_type| ElementRef {
            label: Some("x".to_owned()),
            target: target.to_owned(),
            token_types: vec![token_type],
            is_block: false,
            is_list: false,
            // Mutually exclusive branches.
            cardinality: ChildCardinality {
                min: 0,
                max: Some(1),
            },
            stable_accessor: true,
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
        statement.alts.push(AltModel {
            label: None,
            span: (10, 20),
            refs: vec![token_ref("A", 1), token_ref("B", 2)],
            children: BTreeMap::new(),
            leading_target: None,
        });
        let m = model(vec![statement]);
        let toks = tokens(&[("A", 1), ("B", 2)]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: Some(15),
            site: ActionSite::Body,
            token_types: &toks,
        };

        let error = translate_body("$x.text", &ctx).expect_err("must not translate");
        assert!(error.to_string().contains("cannot translate $x"), "{error}");
    }

    #[test]
    fn translates_ctx_and_text() {
        let m = model(vec![rule("s")]);
        let toks = tokens(&[("ID", 1)]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let text = translate_body("$text", &ctx).expect("translates");
        assert!(
            text.contains("text_interval(action.start_index()"),
            "{text}"
        );
        let tree = translate_body("$ctx.to_string_tree(Some(self))", &ctx).expect("translates");
        assert_eq!(tree, "(&__ctx).to_string_tree(Some(self))");
    }

    #[test]
    fn translates_active_context_child_iterators() {
        let m = model(vec![rule("s"), rule("elseIfStatement")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };

        let translated =
            translate_body("$ctx.elseIfStatement_children()", &ctx).expect("translates");
        assert_eq!(
            translated,
            "__ctx.child_rules(self.base.parse_tree_storage(), self.base.token_store(), 1)"
        );
    }

    #[test]
    fn translates_list_labels_as_lazy_iterators() {
        let mut start = rule("s");
        start.alts.push(AltModel {
            label: None,
            span: (0, 10),
            refs: vec![
                ElementRef {
                    label: Some("args".to_owned()),
                    target: "e".to_owned(),
                    token_types: Vec::new(),
                    is_block: false,
                    is_list: true,
                    cardinality: ChildCardinality { min: 1, max: None },
                    stable_accessor: true,
                    choice_branch: Vec::new(),
                    choice_arity: Vec::new(),
                    choice_spans: Vec::new(),
                    group_spans: Vec::new(),
                    branch_spans: Vec::new(),
                    leading_terminal: true,
                    span: None,
                    branch_local_cardinality: ChildCardinality::ONE,
                    group_local_cardinality: ChildCardinality::ONE,
                },
                ElementRef {
                    label: Some("ids".to_owned()),
                    target: "ID".to_owned(),
                    token_types: vec![1],
                    is_block: false,
                    is_list: true,
                    cardinality: ChildCardinality { min: 1, max: None },
                    stable_accessor: true,
                    choice_branch: Vec::new(),
                    choice_arity: Vec::new(),
                    choice_spans: Vec::new(),
                    group_spans: Vec::new(),
                    branch_spans: Vec::new(),
                    leading_terminal: true,
                    span: None,
                    branch_local_cardinality: ChildCardinality::ONE,
                    group_local_cardinality: ChildCardinality::ONE,
                },
            ],
            children: BTreeMap::new(),
            leading_target: Some("e".to_owned()),
        });
        let m = model(vec![start, rule("e")]);
        let toks = tokens(&[("ID", 1)]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::After,
            token_types: &toks,
        };

        let rules = translate_body("let _: Vec<_> = $args.collect();", &ctx).expect("rule list");
        assert_eq!(rules.matches(".collect()").count(), 1, "{rules}");
        assert!(rules.contains("__ctx.child_rule_trees("), "{rules}");

        let tokens = translate_body("let _: Vec<_> = $ids.collect();", &ctx).expect("token list");
        assert_eq!(tokens.matches(".collect()").count(), 1, "{tokens}");
        assert!(tokens.contains("__ctx.child_tokens("), "{tokens}");
    }

    #[test]
    fn classifies_member_blocks() {
        let body = "i: i32 = 0;\n\
            #[allow(non_snake_case)]\n\
            fn Property(&self) -> bool {\n    true\n}\n\
            struct LeafListener;\n";
        let mut members = MembersModel::default();
        classify_members(body, &mut members).expect("members classify");

        assert_eq!(members.fields.len(), 1);
        assert_eq!(members.fields[0].name, "i");
        assert_eq!(members.fields[0].init, "0");
        assert_eq!(members.impl_items.len(), 1);
        assert!(members.impl_items[0].contains("fn Property"));
        assert_eq!(members.module_items.len(), 1);
    }

    #[test]
    fn dollar_inside_strings_is_left_alone() {
        let m = model(vec![rule("s")]);
        let toks = tokens(&[("ID", 1)]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let body = "writeln!(self.output(), \"{}\", \"$notaref\");";
        assert_eq!(translate_body(body, &ctx).expect("translates"), body);
    }
}
