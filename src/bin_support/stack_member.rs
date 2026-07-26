//! `stack_member` lowering for the `--sem-patterns` DSL (issue #206).
//!
//! Several published grammars keep their lexer state inline in
//! `@lexer::members` — a nesting counter plus a stack or two — and mutate it
//! from inline actions and read it from inline predicates. The bodies are
//! target-language source (C#, Java, …), so they cannot be emitted for the Rust
//! target as written, but their *meaning* is expressible in `SemIR`
//! ([`crate::semir::AStmt::PushMember`] and friends).
//!
//! Rather than teach codegen to parse host-language fragments, a pattern file
//! declares the slot inventory and the generator lowers `match` bodies against
//! it. That keeps the grammar-agnostic boundary intact: the generator still only
//! matches whole declared bodies, and the *mapping* is user-owned data.
//!
//! ```toml
//! # Declare the grammar's member slots, then map each inline body.
//! [[member]]
//! name = "interpolatedStringLevel"
//! kind = "int"
//!
//! [[member]]
//! name = "verbatium"
//! kind = "bool"
//! init = true            # the grammar's `bool verbatium = true;`
//!
//! [[member]]
//! name = "interpolatedVerbatiums"
//! kind = "stack"
//!
//! [[pattern]]
//! match = "interpolatedVerbatiums.Push(true)"
//! lower = "push_member(interpolatedVerbatiums, bool(true))"
//! ```
//!
//! The `lower` grammar this module parses is deliberately tiny and total — it
//! is a constructor syntax for the IR, not an expression language:
//!
//! - `member(NAME)` — scalar slot read
//! - `member_top(NAME)` — stack top, Null when empty
//! - `member_len(NAME)` — stack depth
//! - `int(N)` / `bool(true|false)` — literals
//! - `set_member(NAME, EXPR)` / `add_member(NAME, EXPR)`
//! - `push_member(NAME, EXPR)` / `pop_member(NAME)`
//! - `seq(STMT, STMT, ...)` — the compound bodies real grammars write
//!
//! Slot names resolve through the declared inventory, so a typo is a codegen
//! error naming the unknown slot rather than a silently mis-numbered slot.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io;

/// Whether a declared member slot is a scalar or a stack.
///
/// Scalar and stack slots are numbered in separate namespaces (matching
/// [`crate::semir::MemberEnv`]), so both start at 0.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MemberKind {
    Int,
    Stack,
}

impl MemberKind {
    pub(crate) fn parse(value: &str) -> io::Result<Self> {
        match value {
            // `bool` is an alias for `int`: slots store integers and booleans
            // coerce through truthiness, so a grammar's `bool verbatium;`
            // needs no separate slot kind.
            "int" | "integer" | "bool" | "boolean" => Ok(Self::Int),
            "stack" => Ok(Self::Stack),
            other => Err(invalid_data(format!("unknown member slot kind {other:?}"))),
        }
    }

    const fn describe(self) -> &'static str {
        match self {
            Self::Int => "int",
            Self::Stack => "stack",
        }
    }
}

/// One `[[member]]` declaration from a pattern file.
///
/// `init` carries a scalar slot's declared initial value. Grammars write
/// `private bool verbatium = true;`, and dropping that initializer would leave
/// the slot at 0 — a predicate reading it would then reject input the source
/// grammar accepts, while still reporting itself `translated`. The initializer
/// is metadata rather than something parsed out of the host-language
/// declaration, keeping codegen out of that business.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MemberDeclaration {
    pub(crate) name: String,
    pub(crate) kind: MemberKind,
    pub(crate) init: Option<i64>,
}

/// Slot numbers assigned to the declared members of one grammar.
///
/// Declaration order fixes the numbering, so regenerating an unchanged pattern
/// file produces identical slot ids (and identical generated code).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct MemberSlots {
    slots: BTreeMap<String, (MemberKind, usize)>,
    /// Non-zero scalar initial values, by slot. Zero is the implicit default,
    /// so recording it would emit a redundant seed.
    scalar_inits: BTreeMap<usize, i64>,
}

impl MemberSlots {
    /// Assigns slot numbers to declarations in order, per kind.
    pub(crate) fn assign(declarations: &[MemberDeclaration]) -> io::Result<Self> {
        let mut slots = BTreeMap::new();
        let mut scalar_inits = BTreeMap::new();
        let mut next_int = 0;
        let mut next_stack = 0;
        for declaration in declarations {
            let slot = match declaration.kind {
                MemberKind::Int => {
                    let slot = next_int;
                    next_int += 1;
                    slot
                }
                MemberKind::Stack => {
                    let slot = next_stack;
                    next_stack += 1;
                    slot
                }
            };
            if slots
                .insert(declaration.name.clone(), (declaration.kind, slot))
                .is_some()
            {
                return Err(invalid_data(format!(
                    "duplicate member slot declaration {:?}",
                    declaration.name
                )));
            }
            match (declaration.kind, declaration.init) {
                // A stack's initial contents are not expressible as one scalar,
                // and no real grammar pre-seeds one; reject rather than drop it.
                (MemberKind::Stack, Some(_)) => {
                    return Err(invalid_data(format!(
                        "member slot {:?} is a stack and cannot declare an `init` value",
                        declaration.name
                    )));
                }
                (MemberKind::Int, Some(init)) if init != 0 => {
                    scalar_inits.insert(slot, init);
                }
                _ => {}
            }
        }
        Ok(Self {
            slots,
            scalar_inits,
        })
    }

    /// Resolves a slot name, requiring the given kind.
    fn resolve(&self, name: &str, expected: MemberKind) -> io::Result<usize> {
        let (kind, slot) = self.slots.get(name).copied().ok_or_else(|| {
            invalid_data(format!(
                "unknown member slot {name:?}; declare it with a [[member]] entry"
            ))
        })?;
        if kind != expected {
            return Err(invalid_data(format!(
                "member slot {name:?} is declared {} but used as {}",
                kind.describe(),
                expected.describe()
            )));
        }
        Ok(slot)
    }

    /// Non-zero scalar initial values, as `(slot, value)` in slot order.
    pub(crate) fn scalar_inits(&self) -> impl Iterator<Item = (usize, i64)> + '_ {
        self.scalar_inits
            .iter()
            .map(|(slot, value)| (*slot, *value))
    }

    /// Declared slots in name order, as `(name, kind, slot)`.
    #[cfg(test)]
    pub(crate) fn entries(&self) -> impl Iterator<Item = (&str, MemberKind, usize)> + '_ {
        self.slots
            .iter()
            .map(|(name, (kind, slot))| (name.as_str(), *kind, *slot))
    }
}

/// A `lower` expression resolved against a slot inventory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MemberExpr {
    Bool(bool),
    Int(i64),
    Member(usize),
    MemberTop(usize),
    MemberLen(usize),
    Not(Box<Self>),
}

/// A `lower` statement resolved against a slot inventory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum MemberStmt {
    Set(usize, MemberExpr),
    Add(usize, MemberExpr),
    Push(usize, MemberExpr),
    Pop(usize),
    Seq(Vec<Self>),
}

/// Parses a `lower` expression at the top level of a pattern's `lower` field.
///
/// `None` means "not a member expression", letting the caller fall through to
/// the other pattern lowerings. A bare `bool(...)` / `int(...)` is deliberately
/// declined here: `lower = "bool(false)"` is the long-standing spelling of the
/// constant-false predicate template, and claiming it would silently retarget
/// existing pattern files. Literals are still available *inside* a member
/// expression or statement (`set_member(verbatium, bool(false))`), which is the
/// only place they mean member state.
pub(crate) fn parse_member_expr(
    lower: &str,
    slots: &MemberSlots,
) -> Option<io::Result<MemberExpr>> {
    let (name, _) = split_call(lower.trim())?;
    if matches!(name, "bool" | "int") {
        return None;
    }
    parse_member_operand(lower, slots)
}

/// Parses a member expression in operand position, where literals are members
/// of the grammar (`push_member(s, bool(true))`) rather than constant-predicate
/// spellings.
fn parse_member_operand(lower: &str, slots: &MemberSlots) -> Option<io::Result<MemberExpr>> {
    let lower = lower.trim();
    let (name, body) = split_call(lower)?;
    match name {
        "bool" => Some(match body.trim() {
            "true" => Ok(MemberExpr::Bool(true)),
            "false" => Ok(MemberExpr::Bool(false)),
            other => Err(invalid_data(format!("invalid bool literal {other:?}"))),
        }),
        "int" => Some(
            body.trim()
                .parse()
                .map(MemberExpr::Int)
                .map_err(|error| invalid_data(format!("invalid int literal {body:?}: {error}"))),
        ),
        "member" => Some(
            slots
                .resolve(body.trim(), MemberKind::Int)
                .map(MemberExpr::Member),
        ),
        "member_top" => Some(
            slots
                .resolve(body.trim(), MemberKind::Stack)
                .map(MemberExpr::MemberTop),
        ),
        "member_len" => Some(
            slots
                .resolve(body.trim(), MemberKind::Stack)
                .map(MemberExpr::MemberLen),
        ),
        "not" => Some(
            parse_member_operand(body, slots)
                .unwrap_or_else(|| Err(invalid_data(format!("invalid not() operand {body:?}"))))
                .map(|inner| MemberExpr::Not(Box::new(inner))),
        ),
        _ => None,
    }
}

/// Parses a `lower` statement, or `None` when it is not a member statement.
pub(crate) fn parse_member_stmt(
    lower: &str,
    slots: &MemberSlots,
) -> Option<io::Result<MemberStmt>> {
    let lower = lower.trim();
    let (name, body) = split_call(lower)?;
    match name {
        "pop_member" => Some(
            slots
                .resolve(body.trim(), MemberKind::Stack)
                .map(MemberStmt::Pop),
        ),
        "set_member" | "add_member" | "push_member" => Some(parse_slot_and_expr(name, body, slots)),
        "seq" => Some(parse_seq(body, slots)),
        _ => None,
    }
}

fn parse_slot_and_expr(name: &str, body: &str, slots: &MemberSlots) -> io::Result<MemberStmt> {
    let (slot_name, value) = split_argument(body)
        .ok_or_else(|| invalid_data(format!("{name}() needs a slot name and a value: {body:?}")))?;
    let kind = if name == "push_member" {
        MemberKind::Stack
    } else {
        MemberKind::Int
    };
    let slot = slots.resolve(slot_name.trim(), kind)?;
    let value = parse_member_operand(&value, slots)
        .unwrap_or_else(|| Err(invalid_data(format!("invalid {name}() value {value:?}"))))?;
    Ok(match name {
        "set_member" => MemberStmt::Set(slot, value),
        "add_member" => MemberStmt::Add(slot, value),
        _ => MemberStmt::Push(slot, value),
    })
}

fn parse_seq(body: &str, slots: &MemberSlots) -> io::Result<MemberStmt> {
    let mut statements = Vec::new();
    let mut rest = body.trim().to_owned();
    while !rest.is_empty() {
        let (head, tail) = split_argument(&rest).unwrap_or_else(|| (rest.clone(), String::new()));
        if head.trim().is_empty() {
            return Err(invalid_data(format!("empty seq() element in {body:?}")));
        }
        statements.push(
            parse_member_stmt(&head, slots)
                .unwrap_or_else(|| Err(invalid_data(format!("invalid seq() element {head:?}"))))?,
        );
        tail.trim().clone_into(&mut rest);
    }
    if statements.is_empty() {
        return Err(invalid_data(
            "seq() needs at least one statement".to_owned(),
        ));
    }
    Ok(MemberStmt::Seq(statements))
}

/// Splits `name(body)` into its parts, requiring balanced parentheses.
fn split_call(lower: &str) -> Option<(&str, &str)> {
    let open = lower.find('(')?;
    if !lower.ends_with(')') {
        return None;
    }
    let name = lower[..open].trim();
    if name.is_empty() || !name.bytes().all(|b| b == b'_' || b.is_ascii_alphanumeric()) {
        return None;
    }
    Some((name, &lower[open + 1..lower.len() - 1]))
}

/// Splits the first top-level comma-separated argument off `body`.
///
/// Depth tracking keeps nested calls together: `set_member(a, not(member_top(b)))`
/// must split into `a` and `not(member_top(b))`, not at the inner comma.
fn split_argument(body: &str) -> Option<(String, String)> {
    let mut depth = 0_usize;
    for (index, byte) in body.bytes().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => depth = depth.checked_sub(1)?,
            b',' if depth == 0 => {
                return Some((body[..index].to_owned(), body[index + 1..].to_owned()));
            }
            _ => {}
        }
    }
    None
}

/// Renders a resolved expression as the `SemIr` builder call generated code
/// uses, returning the expression-id variable's initializer.
pub(crate) fn render_member_expr(expr: &MemberExpr, out: &mut String, next: &mut usize) -> String {
    let node = match expr {
        MemberExpr::Bool(value) => format!("antlr4_runtime::semir::PExpr::Bool({value})"),
        MemberExpr::Int(value) => format!("antlr4_runtime::semir::PExpr::Int({value})"),
        MemberExpr::Member(slot) => format!("antlr4_runtime::semir::PExpr::Member({slot})"),
        MemberExpr::MemberTop(slot) => format!("antlr4_runtime::semir::PExpr::MemberTop({slot})"),
        MemberExpr::MemberLen(slot) => format!("antlr4_runtime::semir::PExpr::MemberLen({slot})"),
        MemberExpr::Not(inner) => {
            let inner = render_member_expr(inner, out, next);
            format!("antlr4_runtime::semir::PExpr::Not({inner})")
        }
    };
    let name = format!("__member_expr_{next}");
    *next += 1;
    writeln!(out, "        let {name} = ir.expr({node});")
        .expect("writing to a string cannot fail");
    name
}

/// Renders a resolved statement as `SemIr` builder calls, returning the
/// statement-id variable.
pub(crate) fn render_member_stmt(stmt: &MemberStmt, out: &mut String, next: &mut usize) -> String {
    let node = match stmt {
        MemberStmt::Set(slot, value) => {
            let value = render_member_expr(value, out, next);
            format!("antlr4_runtime::semir::AStmt::SetMember({slot}, {value})")
        }
        MemberStmt::Add(slot, value) => {
            let value = render_member_expr(value, out, next);
            format!("antlr4_runtime::semir::AStmt::AddMember({slot}, {value})")
        }
        MemberStmt::Push(slot, value) => {
            let value = render_member_expr(value, out, next);
            format!("antlr4_runtime::semir::AStmt::PushMember({slot}, {value})")
        }
        MemberStmt::Pop(slot) => format!("antlr4_runtime::semir::AStmt::PopMember({slot})"),
        MemberStmt::Seq(statements) => {
            let ids = statements
                .iter()
                .map(|stmt| render_member_stmt(stmt, out, next))
                .collect::<Vec<_>>()
                .join(", ");
            format!("antlr4_runtime::semir::AStmt::Seq([{ids}].into())")
        }
    };
    let name = format!("__member_stmt_{next}");
    *next += 1;
    writeln!(out, "        let {name} = ir.stmt({node});")
        .expect("writing to a string cannot fail");
    name
}

fn invalid_data(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // insta assertion macros unwrap internal I/O.
mod tests {
    use super::{
        MemberDeclaration, MemberKind, MemberSlots, parse_member_expr, parse_member_stmt,
        render_member_stmt,
    };

    /// The C# interpolation lexer's declared state.
    fn csharp_slots() -> MemberSlots {
        MemberSlots::assign(&[
            MemberDeclaration {
                name: "interpolatedStringLevel".to_owned(),
                kind: MemberKind::Int,
                init: None,
            },
            MemberDeclaration {
                name: "verbatium".to_owned(),
                kind: MemberKind::Int,
                init: None,
            },
            MemberDeclaration {
                name: "interpolatedVerbatiums".to_owned(),
                kind: MemberKind::Stack,
                init: None,
            },
            MemberDeclaration {
                name: "curlyLevels".to_owned(),
                kind: MemberKind::Stack,
                init: None,
            },
        ])
        .expect("declarations should assign")
    }

    #[test]
    fn slots_number_scalars_and_stacks_in_separate_namespaces() {
        let slots = csharp_slots();
        insta::assert_debug_snapshot!(
            "csharp_member_slot_assignment",
            slots.entries().collect::<Vec<_>>()
        );
    }

    #[test]
    fn duplicate_slot_declaration_is_rejected() {
        let error = MemberSlots::assign(&[
            MemberDeclaration {
                name: "x".to_owned(),
                kind: MemberKind::Int,
                init: None,
            },
            MemberDeclaration {
                name: "x".to_owned(),
                kind: MemberKind::Stack,
                init: None,
            },
        ])
        .expect_err("duplicate slot must fail");
        insta::assert_snapshot!("duplicate_member_slot_error", error.to_string());
    }

    /// A declared initializer must reach the generated recognizer: a grammar's
    /// `bool enabled = true;` read by `{enabled}?` otherwise starts at 0 and
    /// rejects input the source grammar accepts. Only non-zero seeds are
    /// recorded, since 0 is already the implicit default.
    #[test]
    fn declared_scalar_initializers_are_recorded_as_seeds() {
        let slots = MemberSlots::assign(&[
            MemberDeclaration {
                name: "enabled".to_owned(),
                kind: MemberKind::Int,
                init: Some(1),
            },
            MemberDeclaration {
                name: "level".to_owned(),
                kind: MemberKind::Int,
                init: Some(7),
            },
            MemberDeclaration {
                name: "explicitZero".to_owned(),
                kind: MemberKind::Int,
                init: Some(0),
            },
            MemberDeclaration {
                name: "undeclared".to_owned(),
                kind: MemberKind::Int,
                init: None,
            },
        ])
        .expect("declarations should assign");
        insta::assert_compact_debug_snapshot!(
            slots.scalar_inits().collect::<Vec<_>>(),
            @"[(0, 1), (1, 7)]"
        );
    }

    /// A stack's initial contents are not expressible as one scalar, so an
    /// `init` on a stack slot is rejected rather than silently dropped.
    #[test]
    fn stack_slots_reject_an_init_value() {
        let error = MemberSlots::assign(&[MemberDeclaration {
            name: "depths".to_owned(),
            kind: MemberKind::Stack,
            init: Some(1),
        }])
        .expect_err("a stack init must be rejected");
        insta::assert_snapshot!(
            error.to_string(),
            @r#"member slot "depths" is a stack and cannot declare an `init` value"#
        );
    }

    #[test]
    fn expressions_resolve_declared_slots() {
        let slots = csharp_slots();
        let parsed = [
            "member(verbatium)",
            "member_top(interpolatedVerbatiums)",
            "member_len(curlyLevels)",
            "not(member_top(interpolatedVerbatiums))",
            "not(member(verbatium))",
        ]
        .into_iter()
        .map(|lower| {
            parse_member_expr(lower, &slots)
                .expect("member expression should match")
                .expect("member expression should resolve")
        })
        .collect::<Vec<_>>();
        insta::assert_debug_snapshot!("member_expression_lowerings", parsed);
    }

    /// A bare literal `lower` stays the constant-predicate template it has
    /// always been; claiming it would silently retarget existing pattern files.
    /// Literals still parse in operand position.
    #[test]
    fn bare_literal_lowerings_are_left_to_the_constant_templates() {
        let slots = csharp_slots();
        assert!(parse_member_expr("bool(true)", &slots).is_none());
        assert!(parse_member_expr("bool(false)", &slots).is_none());
        assert!(parse_member_expr("int(3)", &slots).is_none());

        let nested = parse_member_stmt("push_member(interpolatedVerbatiums, bool(true))", &slots)
            .expect("statement should match")
            .expect("literal operand should resolve");
        insta::assert_compact_debug_snapshot!(nested, @"Push(0, Bool(true))");
    }

    /// The eight C# interpolation sites, lowered. Each is pure `SemIR` — no hook.
    #[test]
    fn csharp_interpolation_bodies_lower_to_statements() {
        let slots = csharp_slots();
        let lowered = [
            // INTERPOLATED_REGULAR_STRING_START
            "seq(add_member(interpolatedStringLevel, int(1)), push_member(interpolatedVerbatiums, bool(false)), set_member(verbatium, bool(false)))",
            // INTERPOLATED_VERBATIUM_STRING_START
            "seq(add_member(interpolatedStringLevel, int(1)), push_member(interpolatedVerbatiums, bool(true)), set_member(verbatium, bool(true)))",
            // OPEN_BRACE_INSIDE
            "push_member(curlyLevels, int(1))",
            // DOUBLE_QUOTE_INSIDE
            "seq(add_member(interpolatedStringLevel, int(-1)), pop_member(interpolatedVerbatiums), set_member(verbatium, member_top(interpolatedVerbatiums)))",
            // CLOSE_BRACE_INSIDE
            "pop_member(curlyLevels)",
        ]
        .into_iter()
        .map(|lower| {
            parse_member_stmt(lower, &slots)
                .expect("member statement should match")
                .expect("member statement should resolve")
        })
        .collect::<Vec<_>>();
        insta::assert_debug_snapshot!("csharp_interpolation_statement_lowerings", lowered);
    }

    #[test]
    fn unknown_and_mistyped_slots_are_named_in_the_error() {
        let slots = csharp_slots();
        let unknown = parse_member_expr("member(nope)", &slots)
            .expect("member() should match")
            .expect_err("unknown slot must fail");
        let mistyped = parse_member_expr("member_top(verbatium)", &slots)
            .expect("member_top() should match")
            .expect_err("scalar used as stack must fail");
        insta::assert_snapshot!(
            "member_slot_resolution_errors",
            format!("{unknown}\n{mistyped}")
        );
    }

    #[test]
    fn non_member_lowerings_decline_so_other_patterns_can_match() {
        let slots = csharp_slots();
        assert!(parse_member_expr("cmp(ne, la(1), token(X))", &slots).is_none());
        assert!(parse_member_stmt("hook", &slots).is_none());
        assert!(parse_member_stmt("true", &slots).is_none());
    }

    #[test]
    fn statements_render_semir_builder_calls() {
        let slots = csharp_slots();
        let stmt = parse_member_stmt(
            "seq(add_member(interpolatedStringLevel, int(-1)), pop_member(interpolatedVerbatiums), set_member(verbatium, member_top(interpolatedVerbatiums)))",
            &slots,
        )
        .expect("statement should match")
        .expect("statement should resolve");
        let mut out = String::new();
        let mut next = 0;
        let root = render_member_stmt(&stmt, &mut out, &mut next);
        insta::assert_snapshot!(
            "csharp_double_quote_inside_rendered",
            format!("{out}        // root: {root}")
        );
    }
}
