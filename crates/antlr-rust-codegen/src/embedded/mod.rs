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

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io;
use std::ops::Range;

use antlr_rust_rs_parser as rust_syntax;
use antlr_rust_rs_parser::{cfg_all_predicate, member_cfg_predicates};

use crate::grammar::action::{ActionReference, action_references as generic_action_references};
use crate::grammar::frontend::SourceId;
use crate::rust_output::rust_identifier_end;
use crate::semantics::template_syntax::{matching_action_brace, skip_ascii_whitespace};

mod antlr4rust;
mod members;
mod model;
mod translate;

pub(crate) use antlr4rust::*;
pub(crate) use members::*;
pub(crate) use model::*;
pub(crate) use translate::*;

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

    #[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
    #[test]
    fn classifies_member_blocks() {
        let body = "i: i32 = 0;\n\
            #[field_attr(nested[0])]\n\
            #[cfg(any())]\n\
            conditional_field: i32 = 1;\n\
            array_field: [u8; AliasTypeParser_ID as usize] =\n\
                [0; AliasTypeParser_ID as usize];\n\
            struct FieldFollower;\n\
            #[allow(non_snake_case)]\n\
            fn Property(&self) -> bool {\n    true\n}\n\
            struct LeafListener;\n\
            use crate::{\n\
                AliasTypeParser_ID,\n\
                CompatParser_ID as Other,\n\
                nested::{self, Item as RenamedItem, Hidden as _},\n\
                r#type,\n\
                *,\n\
            };\n\
            use ::{std::fmt};\n\
            #[cfg(\n\
                any()\n\
            )]\n\
            use crate::CONDITIONAL as AliasTypeParser_CONDITIONAL;\n\
            #[cfg_attr(all(), cfg(any()))]\n\
            use crate::CFG_ATTR as AliasTypeParser_CFG_ATTR;\n";
        let mut members = MembersModel::default();
        classify_members(body, SourceId::new(0), &mut members).expect("members classify");

        assert_eq!(members.fields.len(), 3);
        insta::assert_debug_snapshot!("member_fields", members.fields);
        assert_eq!(members.impl_items.len(), 1);
        assert!(members.impl_items[0].body.contains("fn Property"));
        assert_eq!(members.module_items.len(), 6);
        insta::assert_debug_snapshot!("member_module_symbols", members.module_symbols);
        assert_eq!(
            members.module_symbol_cfgs["FieldFollower"],
            vec![Vec::<String>::new()]
        );
        assert_eq!(
            members.module_import_cfgs["AliasTypeParser_CONDITIONAL"],
            vec![vec!["any()".to_owned()]]
        );
        assert_eq!(
            members.module_import_cfgs["AliasTypeParser_CFG_ATTR"],
            vec![vec!["any(not(all()), any())".to_owned()]]
        );
        assert_eq!(
            members.module_import_cfgs["fmt"],
            vec![Vec::<String>::new()]
        );
    }

    #[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
    #[test]
    fn keeps_only_conditional_attributes_on_member_field_initializers() {
        let attributes = "#[deprecated(note = \"declaration only\")]\n\
                          #[cfg(\n    any()\n)]\n\
                          #[cfg_attr(all(), cfg(any()))]\n";

        insta::assert_snapshot!(
            "member_field_initializer_attributes",
            member_field_initializer_attributes(attributes)
        );
    }

    #[allow(clippy::disallowed_methods)] // insta assertion macros unwrap internal I/O.
    #[test]
    fn classifies_struct_names_by_rust_namespace() {
        let mut members = MembersModel::default();
        classify_members(
            "struct Braced { value: i32 }\n\
             struct Tuple(i32);\n\
             struct Unit;\n\
             struct GenericBraced<T> { value: T }\n\
             struct GenericTuple<T>(T);\n\
             struct WhereBraced<T> where T: Copy { value: T }\n\
             struct WhereTuple<T>(T) where T: Copy;\n\
             struct r#__Antlr4RustInput;\n\
             struct Ωmega;\n",
            SourceId::new(0),
            &mut members,
        )
        .expect("struct members should classify");

        insta::assert_debug_snapshot!(
            members.module_symbol_cfgs.keys().collect::<Vec<_>>(),
            @r#"
        [
            "GenericTuple",
            "Tuple",
            "Unit",
            "WhereTuple",
            "__Antlr4RustInput",
            "Ωmega",
        ]
        "#
        );
    }

    #[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
    #[test]
    fn rewrites_member_compatibility_aliases() {
        let aliases = BTreeSet::from([
            "CompatParser_ID".to_owned(),
            "CompatParser_OTHER".to_owned(),
        ]);
        let translated = translate_member_token_aliases(
            "use self::{CompatParser_ID as Renamed, nested::CompatParser_OTHER};",
            &aliases,
            ANTLR4RUST_TOKEN_ALIAS_MODULE,
        )
        .expect("member use tree should translate");

        assert_eq!(
            translated.source,
            "use self::{__antlr4rust_token_aliases::CompatParser_ID as Renamed, \
             nested::CompatParser_OTHER};"
        );
        assert_eq!(
            translated.token_aliases,
            BTreeSet::from(["CompatParser_ID".to_owned()])
        );
        assert!(translated.direct_alias_imports.is_empty());

        let translated = translate_member_token_aliases(
            "use self::CompatParser_ID;",
            &aliases,
            ANTLR4RUST_TOKEN_ALIAS_MODULE,
        )
        .expect("unrenamed member import should translate");
        assert_eq!(
            translated.source,
            "use self::__antlr4rust_token_aliases::CompatParser_ID;"
        );
        assert_eq!(
            translated.direct_alias_imports,
            BTreeSet::from(["CompatParser_ID".to_owned()])
        );

        let translated = translate_member_token_aliases(
            "fn sees_id(&self) -> bool { CompatParser_ID == Self::ID }",
            &aliases,
            "__antlr4rust_token_aliases_2",
        )
        .expect("member method should translate");
        assert_eq!(
            translated.source,
            "fn sees_id(&self) -> bool { \
             __antlr4rust_token_aliases_2::CompatParser_ID == Self::ID }"
        );
        assert_eq!(
            translated.token_aliases,
            BTreeSet::from(["CompatParser_ID".to_owned()])
        );

        let translated_method = translate_member_token_aliases(
            "fn CompatParser_ID(&self) -> bool { CompatParser_ID == Self::ID }",
            &aliases,
            "__antlr4rust_token_aliases_2",
        )
        .expect("member method names should not bind unqualified body reads");
        let translated_const_argument = translate_member_field_type_token_aliases(
            "Wrapper<{ CompatParser_OTHER as usize }>",
            &aliases,
            "__antlr4rust_token_aliases_2",
        )
        .expect("const generic expressions should lower value aliases");
        insta::assert_snapshot!(
            "antlr4rust_member_declaration_and_const_argument_lowering",
            format!(
                "{}\n---\n{}",
                translated_method.source, translated_const_argument.source
            )
        );
        assert_eq!(
            translated_method.token_aliases,
            BTreeSet::from(["CompatParser_ID".to_owned()])
        );
        assert_eq!(
            translated_const_argument.token_aliases,
            BTreeSet::from(["CompatParser_OTHER".to_owned()])
        );

        let translated = translate_member_field_type_token_aliases(
            "(CompatParser_ID, [u8; CompatParser_OTHER as usize])",
            &aliases,
            "__antlr4rust_token_aliases_2",
        )
        .expect("const expressions in member field types should translate");
        assert_eq!(
            translated.source,
            "(CompatParser_ID, [u8; \
             __antlr4rust_token_aliases_2::CompatParser_OTHER as usize])"
        );
        assert_eq!(
            translated.token_aliases,
            BTreeSet::from(["CompatParser_OTHER".to_owned()])
        );

        let translated = translate_member_token_aliases(
            "impl Helper {\n\
                 const CompatParser_ID: i32 = 1;\n\
                 fn sees_id(&self) -> bool {\n\
                     use std::fmt::Write as _;\n\
                     CompatParser_ID == Self::CompatParser_ID\n\
                 }\n\
             }",
            &aliases,
            ANTLR4RUST_TOKEN_ALIAS_MODULE,
        )
        .expect("local use statements in member impls should remain valid");
        insta::assert_snapshot!("antlr4rust_member_impl_alias_lowering", translated.source);
        assert!(translated.direct_alias_imports.is_empty());
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

    #[test]
    fn macro_rules_metavariables_are_not_antlr_action_references() {
        for body in [
            "macro_rules! m { ($t:ty) => { fn f<'a>(v: &'a $t) {} } }\n$actual",
            "macro_rules! r#match { ($i:ident) => { $i } }\n$actual",
            "macro_rules! λ { ($i:ident) => { $i } }\n$actual",
            "macro_rules! r#λ { ($i:ident) => { $i } }\n$actual",
            "macro_rules /* keyword */ ! /* bang */ value /* name */ \
             { ($i:ident) => { $i } }\n$actual",
            r#"macro_rules! m { ($t:ty) => {{ let _ = "{ $t"; /* } $t */ }} }
$actual"#,
            r#"macro_rules! m { ($t:ty) => {{ /* outer /* inner */ } $ignored */ let _: $t; }} }
$actual"#,
            r"macro_rules! m { ($t:ty) => { let _ = '('; let _ = '\''; } }
$actual",
            r##"macro_rules! m { ($i:ident) => {{ let _ = r#""} $ignored"#; $i }} }
$actual"##,
            r##"macro_rules! m { ($i:ident) => {{ let _ = br#""} $ignored"#; $i }} }
$actual"##,
            r##"macro_rules! m { ($i:ident) => {{ let _ = cr#""} $ignored"#; $i }} }
$actual"##,
        ] {
            let references = action_references(body);
            assert_eq!(references.len(), 1, "{body}");
            assert_eq!(references[0].expression, "$actual", "{body}");
        }
    }

    #[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
    #[test]
    fn lowers_only_supported_antlr4rust_code_tokens() {
        let m = model(vec![rule("s")]);
        let toks = tokens(&[("ID", 1)]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let token_aliases = BTreeSet::from(["CompatParser_ID".to_owned()]);
        let body = r##"
            let _literal = r#"recog.output() _localctx.context()"#;
            // recog.input.peek(1)
            let _raw_recog = r#recog.input.peek(1);
            let _raw_context = r#_localctx.context();
            let _qualified = module::recog.input.peek(1);
            let _before: &'a str = "before";
            let _token = recog /* receiver */ . input.lt(-offset);
            let _probe = Probe { token: recog.input.la(1) };
            let _kind = CompatParser_ID;
            let _after: &'b str = "after";
            _localctx.as_deref().is_some()
        "##
        .trim_end();
        let lowered = translate_parser_body(
            body,
            &ctx,
            "SContext",
            &token_aliases,
            ParserBodyKind::Predicate,
        )
        .expect("supported compatibility surface should lower");

        assert!(lowered.uses_input);
        assert!(lowered.uses_local_context);
        assert_eq!(
            lowered.token_aliases,
            BTreeSet::from(["CompatParser_ID".to_owned()])
        );
        insta::assert_snapshot!(
            "lowers_only_supported_antlr4rust_code_tokens",
            lowered.source
        );
    }

    #[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
    #[test]
    fn preserves_identifier_tokens_for_arbitrary_macros() {
        let m = model(vec![rule("s")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let aliases = BTreeSet::from(["CompatParser_ID".to_owned()]);
        let lowered = translate_parser_body(
            "macro_rules! value { ($i:ident) => { $i } }\n\
             let value = value!(CompatParser_ID);\n\
             take_ident!(CompatParser_ID);\n\
             value == CompatParser_ID && matches!(kind, CompatParser_ID)",
            &ctx,
            "SContext",
            &aliases,
            ParserBodyKind::Predicate,
        )
        .expect("macro identifier arguments should remain opaque");

        assert_eq!(lowered.token_aliases, aliases);
        insta::assert_snapshot!(
            "antlr4rust_arbitrary_macro_identifier_lowering",
            lowered.source
        );
    }

    #[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
    #[test]
    fn preserves_declarations_and_potential_glob_bindings() {
        let m = model(vec![rule("s")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let aliases = BTreeSet::from([
            "CompatParser_ASSOCIATED".to_owned(),
            "CompatParser_FOREIGN".to_owned(),
            "CompatParser_GLOB".to_owned(),
        ]);
        let lowered = translate_parser_body(
            "trait Local { type CompatParser_ASSOCIATED; }\n\
             struct Value;\n\
             impl Local for Value { type CompatParser_ASSOCIATED = u8; }\n\
             unsafe extern \"C\" { static CompatParser_FOREIGN: i32; }\n\
             mod local { pub const CompatParser_GLOB: i32 = 99; }\n\
             use local::*;\n\
             let values = (unsafe { CompatParser_FOREIGN }, CompatParser_GLOB);",
            &ctx,
            "SContext",
            &aliases,
            ParserBodyKind::Action,
        )
        .expect("declarations and glob bindings should remain ordinary Rust");

        assert!(lowered.token_aliases.is_empty());
        insta::assert_snapshot!("antlr4rust_declaration_and_glob_shadowing", lowered.source);
    }

    #[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
    #[test]
    fn preserves_macro_use_shadows_and_cfg_glob_fallbacks() {
        let m = model(vec![rule("s")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let aliases = BTreeSet::from(["CompatParser_ID".to_owned()]);
        let macro_use = translate_parser_body(
            "#[macro_use]\n\
             mod custom {\n\
                 macro_rules! format {\n\
                     (\"{CompatParser_ID}\") => { \"custom\" };\n\
                 }\n\
             }\n\
             let rendered = format!(\"{CompatParser_ID}\");",
            &ctx,
            "SContext",
            &aliases,
            ParserBodyKind::Action,
        )
        .expect("macro-use imports should shadow standard macro lowering");
        let cfg_glob = translate_parser_body(
            "#[cfg(any())]\n\
             use missing::*;\n\
             CompatParser_ID == 1",
            &ctx,
            "SContext",
            &aliases,
            ParserBodyKind::Predicate,
        )
        .expect("cfg-gated glob imports should retain an alias fallback");

        assert!(macro_use.token_aliases.is_empty());
        assert_eq!(cfg_glob.token_aliases, aliases);
        insta::assert_snapshot!(
            "antlr4rust_macro_use_and_cfg_glob_shadowing",
            format!(
                "=== macro use ===\n{}\n=== cfg glob ===\n{}",
                macro_use.source, cfg_glob.source
            )
        );
    }

    #[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
    #[test]
    fn preserves_opaque_compatibility_receivers() {
        let m = model(vec![rule("s")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let lowered = translate_parser_body(
            "macro_rules! identifier_name {\n\
                 ($i:ident) => { stringify!($i) };\n\
             }\n\
             let receiver = stringify!(recog);\n\
             let context = identifier_name!(_localctx);\n\
             #[cfg_attr(any(), allow(recog, _localctx))]\n\
             let attribute = true;\n\
             receiver == \"recog\" && context == \"_localctx\" && attribute",
            &ctx,
            "SContext",
            &BTreeSet::new(),
            ParserBodyKind::Predicate,
        )
        .expect("opaque compatibility receiver tokens should remain target syntax");

        assert!(!lowered.uses_input);
        assert!(!lowered.uses_local_context);
        assert!(lowered.token_aliases.is_empty());
        insta::assert_snapshot!("antlr4rust_opaque_compatibility_receivers", lowered.source);
    }

    #[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
    #[test]
    fn preserves_opaque_type_and_pattern_macro_positions() {
        let m = model(vec![rule("s")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let aliases = BTreeSet::from([
            "CompatParser_EXPR".to_owned(),
            "CompatParser_PATTERN".to_owned(),
            "CompatParser_TYPE".to_owned(),
        ]);
        let lowered = translate_parser_body(
            "macro_rules! alias_type { ($i:ident) => { [u8; $i as usize] }; }\n\
             macro_rules! alias_pattern { ($i:ident) => { $i }; }\n\
             macro_rules! alias_expr { ($i:ident) => { $i }; }\n\
             let _: alias_type!(CompatParser_TYPE) =\n\
                 [0; CompatParser_TYPE as usize];\n\
             let pattern = if let alias_pattern!(CompatParser_PATTERN) =\n\
                 CompatParser_PATTERN {\n\
                 true\n\
             } else {\n\
                 false\n\
             };\n\
             let expression = alias_expr!(CompatParser_EXPR);\n\
             pattern && expression > 0",
            &ctx,
            "SContext",
            &aliases,
            ParserBodyKind::Predicate,
        )
        .expect("type and pattern macros must not receive expression wrappers");

        assert_eq!(
            lowered.token_aliases,
            BTreeSet::from([
                "CompatParser_EXPR".to_owned(),
                "CompatParser_PATTERN".to_owned(),
                "CompatParser_TYPE".to_owned(),
            ])
        );
        insta::assert_snapshot!(
            "antlr4rust_opaque_non_expression_macro_lowering",
            lowered.source
        );
    }

    #[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
    #[test]
    fn keeps_opaque_item_macro_aliases_in_valid_item_scopes() {
        let m = model(vec![rule("s")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let aliases = BTreeSet::from([
            "CompatParser_IMPL_ITEM".to_owned(),
            "CompatParser_MODULE_ITEM".to_owned(),
        ]);
        let lowered = translate_parser_body(
            "macro_rules! define_module_alias {\n\
                 ($i:ident) => { pub(super) fn value() -> i32 { $i } };\n\
             }\n\
             macro_rules! define_impl_alias {\n\
                 ($i:ident) => { fn value() -> i32 { $i } };\n\
             }\n\
             mod nested {\n\
                 define_module_alias!(CompatParser_MODULE_ITEM);\n\
             }\n\
             struct Local;\n\
             impl Local {\n\
                 define_impl_alias!(CompatParser_IMPL_ITEM);\n\
             }\n\
             nested::value() > 0 && Local::value() > 0",
            &ctx,
            "SContext",
            &aliases,
            ParserBodyKind::Predicate,
        )
        .expect("item macros must receive aliases from valid enclosing item scopes");

        assert_eq!(lowered.token_aliases, aliases);
        insta::assert_snapshot!("antlr4rust_opaque_item_macro_lowering", lowered.source);
    }

    #[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
    #[test]
    fn resolves_unbound_format_captures_without_rewriting_the_literal() {
        let m = model(vec![rule("s")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let aliases = BTreeSet::from([
            "CompatParser_ASSERT".to_owned(),
            "CompatParser_CFG".to_owned(),
            "CompatParser_ESCAPED_HEX".to_owned(),
            "CompatParser_ESCAPED_UNICODE".to_owned(),
            "CompatParser_ID".to_owned(),
            "CompatParser_LOCAL".to_owned(),
            "CompatParser_SHADOWED".to_owned(),
            "CompatParser_STD".to_owned(),
        ]);
        let body = "#[cfg(any())]\n\
                    use missing::format;\n\
                    let cfg_fallback = format!(\"{CompatParser_CFG}\");\n\
                    let token = format!(\"{CompatParser_ID}\");\n\
                    let standard = std::format!(\"{CompatParser_STD}\");\n\
                    let escaped_unicode = format!(\"\\u{7b}CompatParser_ESCAPED_UNICODE}\");\n\
                    let escaped_hex = format!(\"\\x7b\\\n\
                        CompatParser_ESCAPED_HEX}\");\n\
                    assert_eq!(helper::<A, B>(), 1, \"{CompatParser_ASSERT}\");\n\
                    let CompatParser_LOCAL = 7;\n\
                    let local = format!(\"{CompatParser_LOCAL}\");\n\
                    macro_rules! format {\n\
                        (\"{CompatParser_SHADOWED}\") => { \"shadowed\" };\n\
                    }\n\
                    let shadowed = format!(\"{CompatParser_SHADOWED}\");\n\
                    token == \"1\" && standard == \"4\"\n\
                        && escaped_unicode == \"2\" && escaped_hex == \"3\"\n\
                        && !cfg_fallback.is_empty()\n\
                        && local == \"7\" && shadowed == \"shadowed\"";
        let lowered =
            translate_parser_body(body, &ctx, "SContext", &aliases, ParserBodyKind::Predicate)
                .expect("format captures should resolve token aliases");

        insta::assert_snapshot!("antlr4rust_format_capture_lowering", lowered.source);
        assert_eq!(
            lowered.token_aliases,
            BTreeSet::from([
                "CompatParser_ASSERT".to_owned(),
                "CompatParser_CFG".to_owned(),
                "CompatParser_ESCAPED_HEX".to_owned(),
                "CompatParser_ESCAPED_UNICODE".to_owned(),
                "CompatParser_ID".to_owned(),
                "CompatParser_STD".to_owned(),
            ])
        );
    }

    #[test]
    fn decodes_ordinary_format_literal_escapes_and_preserves_raw_content() {
        let ordinary = r###""line\nquote\"slash\\hex\x7bunicode\u{7_d}\
                               tail""###;
        let literal = RustLexeme {
            kind: RustLexemeKind::Literal,
            start: 0,
            end: ordinary.len(),
        };
        assert_eq!(
            rust_format_literal_content(ordinary, literal).as_deref(),
            Some("line\nquote\"slash\\hex{unicode}tail")
        );

        let raw = r##"r#"\u{7b}Alias}"#"##;
        let literal = RustLexeme {
            kind: RustLexemeKind::Literal,
            start: 0,
            end: raw.len(),
        };
        assert_eq!(
            rust_format_literal_content(raw, literal).as_deref(),
            Some(r"\u{7b}Alias}")
        );
    }

    #[test]
    fn preserves_matches_pattern_bindings_and_lowers_pattern_constants() {
        let m = model(vec![rule("s")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let aliases = BTreeSet::from([
            "CompatParser_BINDING".to_owned(),
            "CompatParser_FIELD".to_owned(),
            "CompatParser_ID".to_owned(),
        ]);
        let body = "let explicit = matches!(\n\
                        value,\n\
                        CompatParser_BINDING @ Some(CompatParser_ID)\n\
                            if CompatParser_BINDING == Some(CompatParser_ID),\n\
                    );\n\
                    let shorthand = matches!(\n\
                        value,\n\
                        Fields { CompatParser_FIELD }\n\
                            if CompatParser_FIELD == 1,\n\
                    );\n\
                    explicit && shorthand && matches!(kind, CompatParser_ID)";
        let lowered =
            translate_parser_body(body, &ctx, "SContext", &aliases, ParserBodyKind::Predicate)
                .expect("matches pattern bindings should remain ordinary Rust");

        assert!(
            lowered.source.contains(
                "CompatParser_BINDING @ Some(__antlr4rust_token_aliases::CompatParser_ID)"
            )
        );
        assert!(lowered.source.contains(
            "if CompatParser_BINDING == Some(__antlr4rust_token_aliases::CompatParser_ID)"
        ));
        assert!(
            lowered
                .source
                .contains("Fields { CompatParser_FIELD }\nif CompatParser_FIELD == 1"),
            "{}",
            lowered.source
        );
        assert_eq!(
            lowered.token_aliases,
            BTreeSet::from(["CompatParser_ID".to_owned()])
        );
    }

    #[test]
    fn reports_invalid_matches_patterns_during_alias_analysis() {
        let m = model(vec![rule("s")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let aliases = BTreeSet::from(["CompatParser_FIELD".to_owned()]);
        let error = translate_parser_body(
            "matches!(value, Fields { CompatParser_FIELD: })",
            &ctx,
            "SContext",
            &aliases,
            ParserBodyKind::Predicate,
        )
        .expect_err("invalid synthetic patterns must not silently lose bindings");

        assert!(
            error
                .to_string()
                .contains("cannot classify embedded Rust syntax"),
            "{error}"
        );
    }

    #[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
    #[test]
    fn preserves_shadowed_macro_and_attribute_token_trees() {
        let m = model(vec![rule("s")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let aliases = BTreeSet::from(["CompatParser_ID".to_owned()]);
        let lowered = translate_parser_body(
            "macro_rules! assert { (CompatParser_ID) => { true }; }\n\
             let shadowed = assert!(CompatParser_ID);\n\
             let builtin = matches!(kind, CompatParser_ID);\n\
             #[cfg(CompatParser_ID)] let guarded = 1;\n\
             shadowed && builtin && guarded == CompatParser_ID",
            &ctx,
            "SContext",
            &aliases,
            ParserBodyKind::Predicate,
        )
        .expect("opaque token trees should remain valid Rust");

        insta::assert_snapshot!(
            "antlr4rust_opaque_attribute_and_shadowed_macro",
            lowered.source
        );
        assert_eq!(lowered.token_aliases, aliases);
    }

    #[test]
    fn preserves_custom_qualified_macros_and_lowers_standard_paths() {
        let m = model(vec![rule("s")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let aliases = BTreeSet::from(["CompatParser_ID".to_owned()]);
        let lowered = translate_parser_body(
            "my_macros::assert!(CompatParser_ID);\n\
             my_macros::matches!(kind, CompatParser_ID => fallback);\n\
             std::matches!(kind, CompatParser_ID)",
            &ctx,
            "SContext",
            &aliases,
            ParserBodyKind::Predicate,
        )
        .expect("qualified macro token trees should remain opaque");

        assert!(
            lowered
                .source
                .contains("my_macros::assert!(CompatParser_ID)")
        );
        assert!(
            lowered
                .source
                .contains("my_macros::matches!(kind, CompatParser_ID => fallback)")
        );
        assert!(
            lowered
                .source
                .contains("std::matches!(kind, __antlr4rust_token_aliases::CompatParser_ID)")
        );
        assert_eq!(lowered.token_aliases, aliases);
    }

    #[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
    #[test]
    fn preserves_unicode_rust_identifiers() {
        let m = model(vec![rule("s")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let aliases = BTreeSet::from(["CompatParser_ID".to_owned()]);
        let source = "let πrecog = 1;\n\
                      let πCompatParser_ID = 2;\n\
                      πrecog + 1 == πCompatParser_ID";
        let lowered = translate_parser_body(
            source,
            &ctx,
            "SContext",
            &aliases,
            ParserBodyKind::Predicate,
        )
        .expect("Unicode Rust identifiers should remain single lexemes");

        insta::assert_snapshot!("antlr4rust_unicode_identifiers", lowered.source);
        assert!(!lowered.uses_input);
        assert!(lowered.token_aliases.is_empty());
    }

    #[test]
    fn excludes_local_bindings_from_antlr4rust_token_aliases() {
        let m = model(vec![rule("s")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let token_aliases = BTreeSet::from(["CompatParser_ID".to_owned()]);

        for body in [
            "let CompatParser_ID = 7; CompatParser_ID == 7",
            "let CompatParser_ID: i32; CompatParser_ID = 7; CompatParser_ID == 7",
            "let (CompatParser_ID, _) = pair; CompatParser_ID == 7",
            "let Shape::Point(CompatParser_ID) = value else { return false; }; CompatParser_ID == 7",
            "for CompatParser_ID in values { let _ = CompatParser_ID; } true",
            "let closure = |CompatParser_ID| CompatParser_ID; closure(7) == 7",
            "use crate::OTHER as CompatParser_ID; CompatParser_ID == 7",
            "const CompatParser_ID: i32 = 7; CompatParser_ID == 7",
            "static CompatParser_ID: i32 = 7; CompatParser_ID == 7",
            "fn CompatParser_ID() -> i32 { 7 } CompatParser_ID() == 7",
            "fn helper(CompatParser_ID: i32) -> bool { CompatParser_ID == 7 } helper(7)",
            "fn helper<F: Fn(i32) -> i32>(CompatParser_ID: i32, f: F) -> bool { \
             f(CompatParser_ID) == 7 } helper(7, |value| value)",
            "fn helper(CompatParser_ID: i32) -> [i32; { 1 }] { [CompatParser_ID] } \
             helper(7)[0] == 7",
            "fn helper<'CompatParser_ID>(value: &'CompatParser_ID i32) \
             -> &'CompatParser_ID i32 { \
             'CompatParser_ID: loop { break 'CompatParser_ID value; } } true",
            "if let Some(CompatParser_ID) = make(Foo { value: 7 }) { \
             CompatParser_ID == 7 } else { false }",
            "if let Some(CompatParser_ID) = make(Foo { value: 7 }) \
             && CompatParser_ID == 7 { true } else { false }",
            "for CompatParser_ID in make(Foo { values: [7] }).values { \
             let _ = CompatParser_ID; } true",
            "match Some(7) { Some(CompatParser_ID @ _) => CompatParser_ID == 7, None => false }",
            "match Some(7) { Some(ref CompatParser_ID) => *CompatParser_ID == 7, None => false }",
            "match Some(7) { | Some(CompatParser_ID @ _) => \
             CompatParser_ID == 7, None => false }",
            "match Some(7) { None => { false } \
             Some(CompatParser_ID @ _) => { CompatParser_ID == 7 } }",
            "struct CompatParser_ID; let _ = CompatParser_ID; true",
            "union CompatParser_ID { value: i32 } let _ = CompatParser_ID { value: 7 }; true",
            "let r#CompatParser_ID = 7; r#CompatParser_ID == 7",
        ] {
            let lowered = translate_parser_body(
                body,
                &ctx,
                "SContext",
                &token_aliases,
                ParserBodyKind::Predicate,
            )
            .expect("local alias-shaped bindings should remain ordinary Rust");
            assert!(lowered.token_aliases.is_empty(), "{body}");
        }

        for body in [
            "let value = CompatParser_ID; value == 7",
            "CompatParser_ID != 0",
            "if 1 == CompatParser_ID { true } else { false }",
            "while 1 == CompatParser_ID { break; } true",
            "match CompatParser_ID { 1 => true, _ => false }",
            "matches!(kind, CompatParser_ID)",
            "match kind { | CompatParser_ID => true, _ => false }",
            "match kind { Some(CompatParser_ID) => true, _ => false }",
            "if let CompatParser_ID = 7 { true } else { false }",
            "let mut matched = false; while let CompatParser_ID = 7 { \
             matched = true; break; } matched",
        ] {
            let lowered = translate_parser_body(
                body,
                &ctx,
                "SContext",
                &token_aliases,
                ParserBodyKind::Predicate,
            )
            .expect("value references should retain compatibility aliases");
            assert_eq!(
                lowered.token_aliases,
                BTreeSet::from(["CompatParser_ID".to_owned()]),
                "{body}"
            );
        }

        let qualified = translate_parser_body(
            "module::CompatParser_ID == 7",
            &ctx,
            "SContext",
            &token_aliases,
            ParserBodyKind::Predicate,
        )
        .expect("qualified user symbols should remain ordinary Rust");
        assert!(qualified.token_aliases.is_empty());
    }

    #[test]
    fn lowers_relative_paths_that_resolve_to_generated_aliases() {
        let m = model(vec![rule("s")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let token_aliases = BTreeSet::from([
            "CompatParser_DEEP".to_owned(),
            "CompatParser_ID".to_owned(),
            "CompatParser_LOCAL".to_owned(),
            "CompatParser_ROOT".to_owned(),
            "CompatParser_USER".to_owned(),
        ]);
        let body = "let root = self::CompatParser_ROOT;\n\
                    mod nested {\n\
                        pub fn value() -> i32 { super::CompatParser_ID }\n\
                        pub const CompatParser_LOCAL: i32 = 7;\n\
                        pub fn local() -> i32 { self::CompatParser_LOCAL }\n\
                        mod deeper {\n\
                            pub fn value() -> i32 { super::super::CompatParser_DEEP }\n\
                        }\n\
                    }\n\
                    let user = module::CompatParser_USER;\n\
                    root + nested::value() + user > 0";
        let lowered = translate_parser_body(
            body,
            &ctx,
            "SContext",
            &token_aliases,
            ParserBodyKind::Predicate,
        )
        .expect("relative generated-module aliases should lower");

        assert!(
            lowered
                .source
                .contains("self::__antlr4rust_token_aliases::CompatParser_ROOT")
        );
        assert!(
            lowered
                .source
                .contains("super::__antlr4rust_token_aliases::CompatParser_ID")
        );
        assert!(
            lowered
                .source
                .contains("super::super::__antlr4rust_token_aliases::CompatParser_DEEP")
        );
        assert!(lowered.source.contains("self::CompatParser_LOCAL"));
        assert!(lowered.source.contains("module::CompatParser_USER"));
        assert_eq!(
            lowered.token_aliases,
            BTreeSet::from([
                "CompatParser_DEEP".to_owned(),
                "CompatParser_ID".to_owned(),
                "CompatParser_ROOT".to_owned(),
            ])
        );
    }

    #[test]
    fn preserves_struct_literal_fields_when_lowering_alias_values() {
        let m = model(vec![rule("s")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let aliases = BTreeSet::from(["CompatParser_ID".to_owned()]);
        let body = "struct Fields { CompatParser_ID: i32 }\n\
                    let explicit = Fields { CompatParser_ID: CompatParser_ID };\n\
                    let shorthand = Fields { CompatParser_ID };\n\
                    explicit.CompatParser_ID == shorthand.CompatParser_ID";
        let lowered =
            translate_parser_body(body, &ctx, "SContext", &aliases, ParserBodyKind::Predicate)
                .expect("struct literal fields should lower");

        assert_eq!(lowered.token_aliases, aliases);
        assert_eq!(
            lowered
                .source
                .matches(
                    "Fields { CompatParser_ID: \
                     __antlr4rust_token_aliases::CompatParser_ID }"
                )
                .count(),
            2,
            "{}",
            lowered.source
        );
    }

    #[test]
    fn materializes_local_context_after_same_body_attribute_writes() {
        let mut start = rule("s");
        start.attrs.push(AttrDecl {
            name: "value".to_owned(),
            ty: "i32".to_owned(),
        });
        let m = model(vec![start]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let lowered = translate_parser_body(
            "$value = 7; _localctx.as_deref().map(|ctx| ctx.value == 7).unwrap_or(false)",
            &ctx,
            "SContext",
            &BTreeSet::new(),
            ParserBodyKind::Predicate,
        )
        .expect("same-body context reads should lower");

        let write = lowered
            .source
            .find("__attrs.value = 7")
            .expect("attribute write should translate");
        let read = lowered
            .source
            .find("__active_context_view_with_attrs::<SContext")
            .expect("context should materialize at the read");
        assert!(write < read, "{}", lowered.source);
        assert!(!lowered.source.contains("let _localctx"));
    }

    #[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
    #[test]
    fn token_alias_lowering_respects_lexical_binding_scopes() {
        let m = model(vec![rule("s")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let aliases = BTreeSet::from(["ScopeParser_ID".to_owned()]);
        let body = "let before = ScopeParser_ID;\n\
                    { let ScopeParser_ID = 99; let inside = ScopeParser_ID; assert_eq!(inside, 99); }\n\
                    #[cfg(any())]\n\
                    use crate::OTHER as ScopeParser_ID;\n\
                    let after_cfg = ScopeParser_ID;\n\
                    let after = ScopeParser_ID;\n\
                    let inline_const = const { ScopeParser_ID };\n\
                    let const_condition = if let Some(ScopeParser_ID @ _) = const { Some(8) } {\n\
                        ScopeParser_ID == 8\n\
                    } else { false };\n\
                    let closure = |ScopeParser_ID| ScopeParser_ID;\n\
                    let nested = |_outer| |ScopeParser_ID: i32| ScopeParser_ID;\n\
                    let turbofish_match = match Some(5) {\n\
                        Some(ScopeParser_ID @ _) =>\n\
                            Ok::<i32, ()>(ScopeParser_ID).unwrap() == 5,\n\
                        None => false,\n\
                    };\n\
                    let closure_match = match 1 {\n\
                        ScopeParser_ID @ _ =>\n\
                            (move |x, y| ScopeParser_ID + x + y)(2, 3) == 6,\n\
                    };\n\
                    before == after && after_cfg == before && inline_const == before\n\
                        && const_condition\n\
                        && turbofish_match && closure_match\n\
                        && closure(7) == 7 && nested(1)(2) == 2\n\
                        && matches!(before, ScopeParser_ID)";
        let lowered =
            translate_parser_body(body, &ctx, "SContext", &aliases, ParserBodyKind::Predicate)
                .expect("lexically scoped aliases should lower");

        assert_eq!(lowered.token_aliases, aliases);
        insta::assert_snapshot!("antlr4rust_token_alias_lexical_scopes", lowered.source);
    }

    #[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
    #[test]
    fn cfg_gated_local_bindings_select_real_values_or_alias_fallbacks() {
        let m = model(vec![rule("s")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let aliases = BTreeSet::from([
            "CfgParser_ACTIVE_LET".to_owned(),
            "CfgParser_ACTIVE_USE".to_owned(),
            "CfgParser_DUPLICATE".to_owned(),
            "CfgParser_INACTIVE_LET".to_owned(),
            "CfgParser_INACTIVE_USE".to_owned(),
            "CfgParser_ITEM".to_owned(),
            "CfgParser_CONST_GENERIC".to_owned(),
            "CfgParser_CLOSURE".to_owned(),
            "CfgParser_PARAMETER".to_owned(),
            "CfgParser_PATTERN".to_owned(),
            "CfgParser_STAGED".to_owned(),
        ]);
        let body = "#[cfg(all())]\n\
                    use antlr4_runtime::DEFAULT_CHANNEL as CfgParser_ACTIVE_USE;\n\
                    let active_use = CfgParser_ACTIVE_USE;\n\
                    #[cfg(any())]\n\
                    use antlr4_runtime::DEFAULT_CHANNEL as CfgParser_INACTIVE_USE;\n\
                    let inactive_use = CfgParser_INACTIVE_USE;\n\
                    #[cfg(all())]\n\
                    let CfgParser_ACTIVE_LET = 7;\n\
                    let active_let = CfgParser_ACTIVE_LET;\n\
                    #[cfg(any())]\n\
                    let CfgParser_INACTIVE_LET = 8;\n\
                    let inactive_let = CfgParser_INACTIVE_LET;\n\
                    #[cfg(any())]\n\
                    let CfgParser_DUPLICATE = 9;\n\
                    #[cfg(any())]\n\
                    use antlr4_runtime::DEFAULT_CHANNEL as CfgParser_DUPLICATE;\n\
                    let duplicate = CfgParser_DUPLICATE;\n\
                    #[cfg(any())]\n\
                    let CfgParser_STAGED = 9;\n\
                    let staged_before = CfgParser_STAGED;\n\
                    #[cfg(all())]\n\
                    let CfgParser_STAGED = 10;\n\
                    let staged_after = CfgParser_STAGED;\n\
                    fn cfg_parameter(\n\
                        #[cfg(any())] CfgParser_PARAMETER: i32,\n\
                    ) -> i32 {\n\
                        CfgParser_PARAMETER\n\
                    }\n\
                    let parameter = cfg_parameter();\n\
                    #[cfg(any())]\n\
                    const CfgParser_ITEM: i32 = 11;\n\
                    let item = CfgParser_ITEM;\n\
                    fn cfg_const_generic<\n\
                        #[cfg(any())] const CfgParser_CONST_GENERIC: usize,\n\
                    >() -> i32 {\n\
                        CfgParser_CONST_GENERIC\n\
                    }\n\
                    let const_generic = cfg_const_generic();\n\
                    let cfg_closure = |\n\
                        #[cfg(any())] CfgParser_CLOSURE: i32,\n\
                    | CfgParser_CLOSURE;\n\
                    let closure = cfg_closure();\n\
                    struct CfgFields {\n\
                        #[cfg(any())]\n\
                        CfgParser_PATTERN: i32,\n\
                    }\n\
                    let CfgFields {\n\
                        #[cfg(any())]\n\
                        CfgParser_PATTERN,\n\
                    } = CfgFields {};\n\
                    let pattern = CfgParser_PATTERN;\n\
                    active_use == 0 && inactive_use > 0\n\
                        && active_let == 7 && inactive_let > 0 && duplicate > 0\n\
                        && staged_before > 0 && staged_after == 10 && parameter > 0\n\
                        && item > 0 && const_generic > 0 && closure > 0\n\
                        && pattern > 0";
        let lowered =
            translate_parser_body(body, &ctx, "SContext", &aliases, ParserBodyKind::Predicate)
                .expect("cfg-gated bindings should retain active values and inactive fallbacks");

        assert_eq!(lowered.token_aliases, aliases);
        insta::assert_snapshot!(
            "antlr4rust_cfg_gated_local_binding_fallbacks",
            lowered.source
        );
    }

    #[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
    #[test]
    fn cfg_gated_match_bindings_use_expression_fallbacks() {
        let m = model(vec![rule("s")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let aliases = BTreeSet::from(["CfgParser_MATCH".to_owned()]);
        let body = "struct Fields {\n\
                        #[cfg(any())]\n\
                        CfgParser_MATCH: i32,\n\
                    }\n\
                    let fields = Fields {};\n\
                    let matched = Some(match fields {\n\
                        Fields {\n\
                            #[cfg(any())]\n\
                            CfgParser_MATCH,\n\
                        } if CfgParser_MATCH > 0 => true,\n\
                        _ => false,\n\
                    });\n\
                    matched == Some(true)";
        let lowered =
            translate_parser_body(body, &ctx, "SContext", &aliases, ParserBodyKind::Predicate)
                .expect("cfg-gated match bindings should expose inactive alias fallbacks");

        assert_eq!(lowered.token_aliases, aliases);
        insta::assert_snapshot!(
            "antlr4rust_cfg_gated_match_binding_fallback",
            lowered.source
        );
    }

    #[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
    #[test]
    fn cfg_gated_for_bindings_use_body_fallbacks() {
        let m = model(vec![rule("s")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let aliases = BTreeSet::from(["CfgParser_FOR".to_owned()]);
        let body = "struct Fields {\n\
                        #[cfg(any())]\n\
                        CfgParser_FOR: i32,\n\
                    }\n\
                    let mut found = false;\n\
                    for Fields {\n\
                        #[cfg(any())]\n\
                        CfgParser_FOR,\n\
                    } in [Fields {}] {\n\
                        found = CfgParser_FOR > 0;\n\
                    }\n\
                    found";
        let lowered =
            translate_parser_body(body, &ctx, "SContext", &aliases, ParserBodyKind::Predicate)
                .expect("cfg-gated for bindings should expose body-local alias fallbacks");

        assert_eq!(lowered.token_aliases, aliases);
        insta::assert_snapshot!("antlr4rust_cfg_gated_for_binding_fallback", lowered.source);
    }

    #[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
    #[test]
    fn cfg_gated_matches_bindings_use_expression_fallbacks() {
        let m = model(vec![rule("s")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };
        let aliases = BTreeSet::from(["CfgParser_MATCHES".to_owned()]);
        let body = "struct Fields {\n\
                        #[cfg(any())]\n\
                        CfgParser_MATCHES: i32,\n\
                    }\n\
                    let direct = matches!(\n\
                        Fields {},\n\
                        Fields {\n\
                            #[cfg(any())]\n\
                            CfgParser_MATCHES,\n\
                        } if CfgParser_MATCHES > 0,\n\
                    );\n\
                    let standard = std::matches!(\n\
                        Fields {},\n\
                        Fields {\n\
                            #[cfg(any())]\n\
                            CfgParser_MATCHES,\n\
                        } if CfgParser_MATCHES > 0,\n\
                    );\n\
                    direct && standard";
        let lowered =
            translate_parser_body(body, &ctx, "SContext", &aliases, ParserBodyKind::Predicate)
                .expect("cfg-gated matches bindings should expose expression alias fallbacks");

        assert_eq!(lowered.token_aliases, aliases);
        insta::assert_snapshot!(
            "antlr4rust_cfg_gated_matches_binding_fallback",
            lowered.source
        );
    }

    #[test]
    fn lexer_bodies_reject_parser_only_compatibility_receivers() {
        let error = validate_lexer_body_compatibility_receivers("recog.input.la(1)")
            .expect_err("parser input compatibility is unavailable in lexers");
        assert!(
            error
                .to_string()
                .contains("`recog.input` is only supported in embedded parser bodies"),
            "{error}"
        );
        let error = validate_lexer_body_compatibility_receivers("_localctx.as_deref().is_some()")
            .expect_err("parser context compatibility is unavailable in lexers");
        assert!(
            error
                .to_string()
                .contains("`_localctx` is only supported in embedded parser bodies"),
            "{error}"
        );
    }

    #[test]
    fn rejects_unknown_antlr4rust_members_before_rust_compilation() {
        let m = model(vec![rule("s")]);
        let toks = tokens(&[]);
        let ctx = TranslationCtx {
            model: &m,
            rule_index: 0,
            body_offset: None,
            site: ActionSite::Body,
            token_types: &toks,
        };

        for (body, expected) in [
            ("recog", "unsupported bare `recog` reference"),
            ("recog.output()", "unsupported `recog.output` member"),
            (
                "recog.input.peek(1)",
                "unsupported `recog.input.peek` accessor",
            ),
            (
                "recog.input.la()",
                "`recog.input.la` requires one offset argument",
            ),
            (
                "recog.input.la(1, 2)",
                "`recog.input.la` accepts exactly one offset argument",
            ),
            (
                "recog.input.lt(1, nested(2, 3))",
                "`recog.input.lt` accepts exactly one offset argument",
            ),
            (
                "_localctx.context()",
                "unsupported `_localctx.context` accessor",
            ),
        ] {
            let error = translate_parser_body(
                body,
                &ctx,
                "SContext",
                &BTreeSet::new(),
                ParserBodyKind::Predicate,
            )
            .expect_err("unknown compatibility surface must fail");
            assert!(error.to_string().contains(expected), "{error}");
        }

        translate_parser_body(
            "recog.input.la((1, 2).0)",
            &ctx,
            "SContext",
            &BTreeSet::new(),
            ParserBodyKind::Predicate,
        )
        .expect("a nested comma remains one offset argument");
        translate_parser_body(
            "recog.input.la(Option::<i32>::Some(1).map_or::<i32, _>(0, |value| value))",
            &ctx,
            "SContext",
            &BTreeSet::new(),
            ParserBodyKind::Predicate,
        )
        .expect("a turbofish comma remains inside one offset argument");
    }
}
