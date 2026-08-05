#![allow(clippy::disallowed_methods)] // insta assertion macros unwrap internal I/O.
#[allow(clippy::wildcard_imports)]
use super::support::*;

/// Issue #267: the exact embedded-Rust forms emitted by the pinned C and Java
/// grammars-v4 transforms lower onto native token and active-context APIs.
fn antlr4rust_reviewed_lexical_edges(source: &str) -> String {
    source
        .lines()
        .filter(|line| {
            [
                "AliasCollisionParser_METHOD_NAME",
                "AliasCollisionParser_CONST_EXPRESSION",
                "AliasCollisionParser_SHADOWED_MACRO",
                "AliasCollisionParser_ATTRIBUTE",
                "πrecog",
                "πAliasCollisionParser_UNICODE",
                "AliasCollisionParser_ACTION_CFG",
                "AliasCollisionParser_CONST_BLOCK",
                "AliasCollisionParser_ASSOCIATED_CONST",
                "AliasCollisionParser_MATCHES_BINDING",
                "AliasCollisionParser_PRECISE_CAPTURE",
                "AliasCollisionParser_FORMAT_CAPTURE",
                "AliasCollisionParser_ESCAPED_FORMAT",
                "AliasCollisionParser_CONTINUED_FORMAT",
                "AliasCollisionParser_FORMAT_LOCAL",
                "AliasCollisionParser_QUALIFIED_MACRO",
                "AliasCollisionParser_CONST_CHAIN",
                "AliasCollisionParser_TYPE_ONLY",
                "AliasCollisionParser_VALUE_IMPORT",
                "AliasCollisionParser_UNSAFE_EXTERN",
                "AliasCollisionParser_TURBOFISH",
                "AliasCollisionParser_STANDARD_FORMAT",
                "AliasCollisionParser_SAFE_FOREIGN",
                "AliasCollisionParser_RAW_LIFETIME",
                "AliasCollisionParser_OPAQUE_MACRO",
                "AliasCollisionParser_CUSTOM_MATCHES",
                "AliasCollisionParser_CLOSURE_MATCH",
                "AliasCollisionParser_ACTIVE_CFG_USE",
                "AliasCollisionParser_INACTIVE_CFG_USE",
                "AliasCollisionParser_ACTIVE_CFG_LET",
                "AliasCollisionParser_INACTIVE_CFG_LET",
                "AliasCollisionParser_DUPLICATE_CFG",
                "AliasCollisionParser_IMPORTED_MACRO",
                "AliasCollisionParser_TYPE_MACRO",
                "AliasCollisionParser_PATTERN_MACRO",
                "AliasCollisionParser_CFG_FORMAT",
                "AliasCollisionParser_RAW_MACRO_NAME",
                "AliasCollisionParser_STAGED_CFG",
                "AliasCollisionParser_CFG_PARAMETER",
                "AliasCollisionParser_RAW_STRING_MACRO",
                "AliasCollisionParser_PATTERN_CFG",
                "AliasCollisionParser_FOR_PATTERN_CFG",
                "AliasCollisionParser_MATCHES_PATTERN_CFG",
                "AliasCollisionParser_ASSOCIATED_BOUND",
                "AliasCollisionParser_PARENT_MODULE",
                "AliasCollisionParser_CFG_ITEM",
                "AliasCollisionParser_CFG_CONST_GENERIC",
                "AliasCollisionParser_CFG_CLOSURE",
                "AliasCollisionParser_UNICODE_MACRO_NAME",
                "AliasCollisionParser_ASSOCIATED_TYPE",
                "AliasCollisionParser_FOREIGN_STATIC",
                "AliasCollisionParser_GLOB_IMPORT",
                "AliasCollisionParser_MACRO_USE",
                "AliasCollisionParser_GE_NO_STRUCT",
                "standard_qualified_macro_ok",
                "c_strings_ok",
                "placeholder_lifetime",
                "raw_reference_ok",
                "nested_raw_reference_ok",
                "exponent_underscore_ok",
                "one_sided_range_patterns_ok",
                "attributed_binder_ok",
                "const_generic_default_ok",
                "safe_identifier_ok",
                "unicode_escape_underscores_ok",
                "multiple_match_inner_attrs_ok",
                "non_bmp_identifier_ok",
                "underscore_assignment_ok",
                "opaque_receiver_tokens_ok",
                "opaque_attribute_receiver_tokens_ok",
                "if_let_constant_ok",
                "while_let_constant_ok",
                "cfg_disabled_format_ok",
                "shadowed_standard_path_ok",
                "literal_const_arguments_ok",
                "impl_inner_attribute_ok",
                "raw_macro_name_ok",
                "staged_cfg_before",
                "staged_cfg_after",
                "cfg_parameter_ok",
                "raw_string_macro_ok",
                "nonleading_let_chain_ok",
                "associated_type_bound_ok",
                "match_pattern_cfg_ok",
                "for_pattern_cfg_ok",
                "matches_pattern_cfg_ok",
                "escaped_format_capture_ok",
                "continued_format_capture_ok",
                "reviewed_foreign_syntax_ok",
                "foreign_static_binding_ok",
                "associated_type_declaration_ok",
                "glob_import_binding_ok",
                "macro_use_shadow_ok",
                "ge_no_struct_ok",
                "numeric_tuple_pattern_ok",
                "pattern_cfg_ok",
                "parent_module_alias_ok",
                "tuple_field_ok",
                "pub_self_ok",
                "pub_in_self_ok",
                "empty_turbofish_ok",
                "empty_where_ok",
                "cfg_item_ok",
                "cfg_const_generic_ok",
                "cfg_closure_ok",
                "unicode_macro_name_ok",
                "commented_macro_header_ok",
                "module_item_macro_ok",
                "impl_item_macro_ok",
            ]
            .iter()
            .any(|needle| line.contains(needle))
        })
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n")
}

#[track_caller]
fn assert_antlr4rust_reviewed_rust_syntax(source: &str) {
    for expected in [
        "AliasCollisionParser_MATCHES_BINDING @ Some(_)",
        "if AliasCollisionParser_MATCHES_BINDING",
        "Some(AliasCollisionParser_TURBOFISH @ _)",
        "Ok::<i32, ()>(AliasCollisionParser_TURBOFISH)",
        "AliasCollisionParser_CLOSURE_MATCH @ _",
        "|x, y| AliasCollisionParser_CLOSURE_MATCH",
        "if let __antlr4rust_token_aliases_2::AliasCollisionParser_MODULE = MODULE",
        "while let __antlr4rust_token_aliases_2::AliasCollisionParser_MODULE = MODULE",
        "alias_type!(AliasCollisionParser_TYPE_MACRO)",
        "alias_pattern!(AliasCollisionParser_PATTERN_MACRO)",
        "pair.0.1",
        "pub(self) fn helper()",
        "pub(in self) fn helper()",
        "empty_turbofish_helper::<>()",
        "fn empty_where_helper() -> i32 where {",
        "unsafe static UNSAFE_FOREIGN: i32",
        "fn attributed_variadic(#[allow(unused)] ...)",
        "unsafe extern \"C\" fn(#[allow(unused)] ...)",
        "TuplePattern { 0: Some(value), 1: _ }",
        "λ!(AliasCollisionParser_UNICODE_MACRO_NAME)",
        "& raw const raw_reference_value",
        "*&raw const raw_reference_value",
        "1e_1 == 10.0",
        "..=1 => false",
        "2.. => true",
        "for<#[cfg(all())] 'a> fn(&'a i32)",
        "struct DefaultedConst<const N: usize = 3>",
        "fn safe(safe: i32)",
        r#""\u{00_E6}" == "\u{00E6}""#,
        "#![allow(dead_code)]",
        "let 𞤀 = 47",
        "_ = {",
        "stringify!(recog)",
        "receiver_name!(_localctx)",
        "allow(recog, _localctx)",
        "} if AliasCollisionParser_PATTERN_CFG == Self::PATTERN_CFG",
        "define_module_alias!(AliasCollisionParser_OPAQUE_MACRO)",
        "define_impl_alias!(AliasCollisionParser_OPAQUE_MACRO)",
        "type AliasCollisionParser_ASSOCIATED_TYPE;",
        "type AliasCollisionParser_ASSOCIATED_TYPE = u8;",
        "static AliasCollisionParser_FOREIGN_STATIC: i32;",
        "use glob_values::*;",
        "format!(\"{AliasCollisionParser_MACRO_USE}\")",
        "Self::GE_NO_STRUCT >= \
         __antlr4rust_token_aliases_2::AliasCollisionParser_GE_NO_STRUCT",
    ] {
        assert!(
            source.contains(expected),
            "pattern bindings and their reads must remain ordinary Rust bindings: {expected}"
        );
    }
    let attributed_shorthand = source
        .split_once("let attributed_shorthand = AliasFields {")
        .expect("attributed shorthand initializer")
        .1
        .split_once("};")
        .expect("end of attributed shorthand initializer")
        .0;
    assert!(
        attributed_shorthand.contains("#[allow(unused)]")
            && attributed_shorthand.contains(
                "AliasCollisionParser_FIELD: \
                 __antlr4rust_token_aliases_2::AliasCollisionParser_FIELD"
            ),
        "attributed shorthand fields must retain attributes while expanding alias values"
    );
}

#[track_caller]
fn assert_antlr4rust_import_namespace_fallbacks(source: &str) {
    assert!(
        source.contains("pub(super) use super::AliasCollisionParser_EOF;")
            && source.contains(
                "pub(crate) const AliasCollisionParser_EOF: i32 = \
                 antlr4_runtime::TOKEN_EOF;"
            ),
        "a value import must override a namespace-safe compatibility fallback"
    );
    assert!(
        source.contains("#[cfg(any())]")
            && source.contains("pub(super) use super::AliasCollisionParser_CFG;")
            && source.contains("pub(crate) const AliasCollisionParser_CFG: i32"),
        "a cfg-disabled member import must retain a compatibility fallback"
    );
    assert!(
        source.contains("use std::fmt::Result as AliasCollisionParser_TYPE_ONLY;")
            && source.contains("pub(super) use super::AliasCollisionParser_TYPE_ONLY;")
            && source.contains("pub(crate) const AliasCollisionParser_TYPE_ONLY: i32"),
        "type-only imports must coexist with the token-value fallback"
    );
    assert!(
        source.contains("use antlr4_runtime::TOKEN_EOF as AliasCollisionParser_VALUE_IMPORT;")
            && source.contains("pub(super) use super::AliasCollisionParser_VALUE_IMPORT;")
            && source.contains("pub(crate) const AliasCollisionParser_VALUE_IMPORT: i32"),
        "value imports must override the token-value fallback"
    );
}

#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
#[test]
fn antlr4rust_transform_surface_compiles_and_matches_native_behavior() {
    let temp = temporary_directory("antlr4rust-compat");
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/antlr4-rust-gen/antlr4rust-compat");
    let out = temp.path().join("generated");

    let output = run_antlr4_rust_gen(&[
        fixtures.join("CCompat.g4").as_os_str(),
        fixtures.join("JavaCompat.g4").as_os_str(),
        fixtures.join("AliasOnly.g4").as_os_str(),
        fixtures.join("AliasCollision.g4").as_os_str(),
        OsStr::new("--actions"),
        OsStr::new("embedded"),
        OsStr::new("--sem-unknown"),
        OsStr::new("error"),
        OsStr::new("--require-full-semantics"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );

    let manifest =
        fs::read_to_string(out.join("semantics.json")).expect("manifest should be emitted");
    insta::assert_snapshot!("antlr4rust_compat_semantics_manifest", manifest);

    let c_parser =
        fs::read_to_string(out.join("c_compat_parser.rs")).expect("C parser should be emitted");
    let java_parser = fs::read_to_string(out.join("java_compat_parser.rs"))
        .expect("Java parser should be emitted");
    let alias_parser = fs::read_to_string(out.join("alias_only_parser.rs"))
        .expect("alias-only parser should be emitted");
    let alias_collision_parser = fs::read_to_string(out.join("alias_collision_parser.rs"))
        .expect("alias-collision parser should be emitted");
    assert!(
        !alias_parser.contains("__Antlr4RustInput"),
        "token-alias-only bodies should not emit the input facade"
    );
    assert_antlr4rust_import_namespace_fallbacks(&alias_collision_parser);
    assert!(
        !alias_collision_parser.contains("const AliasCollisionParser_ID"),
        "a user member symbol must suppress the colliding token alias"
    );
    assert!(
        alias_collision_parser.contains("const AliasCollisionParser_MODULE"),
        "a renamed import path must not suppress the original compatibility alias"
    );
    assert!(
        !alias_collision_parser.contains("const AliasCollisionParser_LOCAL: i32"),
        "a body-local binding must suppress the colliding token alias"
    );
    assert!(
        alias_collision_parser.contains(
            "use self::{__antlr4rust_token_aliases_2::AliasCollisionParser_MEMBER_ONLY as \
             RenamedMemberOnly};"
        ),
        "member-only compatibility imports must target the generated alias namespace"
    );
    assert!(
        alias_collision_parser.contains(
            "let before_scope = \
             __antlr4rust_token_aliases_2::AliasCollisionParser_SCOPE;"
        ) && alias_collision_parser.contains(
            "let after_scope = \
             __antlr4rust_token_aliases_2::AliasCollisionParser_SCOPE;"
        ) && alias_collision_parser.contains("let AliasCollisionParser_SCOPE = 99;"),
        "compatibility aliases must respect nested lexical bindings"
    );
    assert!(
        alias_collision_parser.contains("mod __antlr4rust_token_aliases_2 {")
            && alias_collision_parser.contains("struct __antlr4rust_token_aliases;"),
        "the generated alias module must avoid user member symbols"
    );
    assert!(
        alias_collision_parser
            .contains("__antlr4rust_token_aliases_2::AliasCollisionParser_MODULE == Self::MODULE"),
        "member methods must lower compatibility aliases"
    );
    assert!(
        alias_collision_parser
            .contains("marker: __antlr4rust_token_aliases_2::AliasCollisionParser_FIELD_INIT"),
        "member field initializers must lower compatibility aliases"
    );
    assert!(
        alias_collision_parser.contains(
            "field_type: [u8; \
             __antlr4rust_token_aliases_2::AliasCollisionParser_FIELD_TYPE as usize],"
        ) && alias_collision_parser.contains(
            "field_type: [0; \
             __antlr4rust_token_aliases_2::AliasCollisionParser_FIELD_TYPE as usize],"
        ),
        "member field type const expressions and initializers must lower compatibility aliases"
    );
    let alias_collision_lines = alias_collision_parser.lines().collect::<Vec<_>>();
    let conditional_field_attributes = alias_collision_lines
        .windows(2)
        .filter(|lines| lines[1].contains("conditional_field:"))
        .map(|lines| format!("{}\n{}", lines[0].trim(), lines[1].trim()))
        .collect::<Vec<_>>()
        .join("\n---\n");
    insta::assert_snapshot!(
        "antlr4rust_member_field_attributes",
        conditional_field_attributes
    );
    insta::assert_snapshot!(
        "antlr4rust_reviewed_lexical_edges",
        antlr4rust_reviewed_lexical_edges(&alias_collision_parser)
    );
    assert!(
        alias_collision_parser.contains("impl<const AliasCollisionParser_IMPL_CONST: usize>")
            && alias_collision_parser.contains("AliasCollisionParser_IMPL_CONST\n        }")
            && !alias_collision_parser.contains("const AliasCollisionParser_IMPL_CONST: i32"),
        "impl const-generic bindings must remain local to the member item"
    );
    assert_antlr4rust_reviewed_rust_syntax(&alias_collision_parser);
    assert!(
        alias_collision_parser.contains("struct __Antlr4RustContext;")
            && alias_collision_parser.contains("struct __Antlr4RustContext_2<T>(T);")
            && alias_collision_parser.contains(".map(__Antlr4RustContext_2)"),
        "the generated context wrapper must avoid user member symbols"
    );
    assert!(
        alias_collision_parser.contains("struct r#__Antlr4RustInput;")
            && alias_collision_parser.contains("struct __Antlr4RustInput_2<'a, L: TokenSource>")
            && alias_collision_parser.contains("__Antlr4RustInput_2(self.base.token_stream())")
            && alias_collision_parser.contains("struct __Antlr4RustTokenView;")
            && alias_collision_parser
                .contains("struct __Antlr4RustTokenView_2<'a>(antlr4_runtime::TokenView<'a>)")
            && alias_collision_parser.contains("token.map(__Antlr4RustTokenView_2)"),
        "the generated input facade must avoid user member symbols"
    );
    assert!(
        alias_collision_parser.contains("const AliasCollisionParser_NAMED")
            && !alias_collision_parser.contains("use super::AliasCollisionParser_NAMED;"),
        "a braced struct must not suppress the same-named value alias"
    );
    assert!(
        alias_collision_parser
            .contains("use self::__antlr4rust_token_aliases_2::AliasCollisionParser_DIRECT;")
            && alias_collision_parser.contains("const AliasCollisionParser_DIRECT")
            && !alias_collision_parser.contains("use super::AliasCollisionParser_DIRECT"),
        "unrenamed self imports must resolve to a defined compatibility alias"
    );
    assert!(
        alias_collision_parser.contains("use std::fmt::Write as _;")
            && alias_collision_parser
                .contains("__antlr4rust_token_aliases_2::AliasCollisionParser_MODULE == MODULE"),
        "member impls must preserve local uses while lowering compatibility aliases"
    );
    let unexpected_local_aliases = [
        "MATCH",
        "ARM",
        "CHAIN",
        "IF",
        "FOR",
        "PARAM",
        "BRACED_PARAM",
        "FORMAT_LOCAL",
        "CONST_CHAIN",
        "TURBOFISH",
        "CLOSURE_MATCH",
    ]
    .into_iter()
    .filter(|name| alias_collision_parser.contains(&format!("const AliasCollisionParser_{name}:")))
    .collect::<Vec<_>>();
    assert!(
        unexpected_local_aliases.is_empty(),
        "macro identifiers, lifetimes, labels, and local bindings must not request compatibility aliases: {unexpected_local_aliases:?}"
    );
    assert!(
        alias_collision_parser.contains(
            "AliasCollisionParser_FIELD: \
             __antlr4rust_token_aliases_2::AliasCollisionParser_FIELD"
        ),
        "struct field names must remain intact while alias values are qualified"
    );
    assert!(
        java_parser
            .contains("pub fn r#type(&self) -> Option<__Antlr4RustContext<TypeContext<'a>>>")
            && java_parser.contains("self.0.r#type().ok().map(__Antlr4RustContext)")
            && java_parser
                .contains("pub fn self_(&self) -> Option<__Antlr4RustContext<SelfContext<'a>>>")
            && java_parser.contains("self.0.self_().ok().map(__Antlr4RustContext)")
            && java_parser
                .contains("pub fn r#type(&self) -> Result<TypeContext<'a>, MissingChildError>")
            && java_parser
                .contains("pub fn self_(&self) -> Result<SelfContext<'a>, MissingChildError>"),
        "keyword compatibility getters must coexist with the native fallible surface"
    );
    let unrelated_context_tail = java_parser
        .split_once("pub struct UnrelatedContext")
        .expect("unrelated context should be emitted")
        .1;
    let unrelated_context_end = unrelated_context_tail
        .find("\nantlr4_runtime::__antlr4_rust_context!")
        .or_else(|| unrelated_context_tail.find("\n/// Checks generated required-child invariants"))
        .expect("the next generated surface should delimit the unrelated context");
    let unrelated_context = &unrelated_context_tail[..unrelated_context_end];
    insta::assert_snapshot!(
        "antlr4rust_unrelated_context_surface",
        unrelated_context
            .lines()
            .filter(|line| line.trim_start().starts_with("pub fn "))
            .map(str::trim)
            .collect::<Vec<_>>()
            .join("\n")
    );
    let live_context_type =
        "__active_context_view_with_attrs::<LiveAttributesContext<'_, __ActiveParserContext>>";
    assert!(
        java_parser.contains(live_context_type),
        "active context must retain the live-attributes context type"
    );
    let live_context = java_parser
        .find(live_context_type)
        .expect("live-attribute predicate should materialize its active context");
    let live_context_end = java_parser[live_context..]
        .find(").map(")
        .expect("active-context call should terminate");
    insta::assert_snapshot!(
        "antlr4rust_live_attributes_active_context_call",
        &java_parser[live_context..=live_context + live_context_end]
    );
    assert!(
        !java_parser.contains("let _localctx"),
        "active contexts must materialize at each use so same-body attribute writes stay visible"
    );
    let excerpt = |source: &str| {
        source
            .lines()
            .filter(|line| {
                [
                    "__Antlr4RustInput",
                    "__Antlr4RustTokenView",
                    "CCompatParser_",
                    "JavaCompatParser_",
                    "AliasOnlyParser_",
                    "recordComponent_all",
                    "pub fn IDENTIFIER_all",
                    "pub fn IDENTIFIER",
                    "pub fn ELLIPSIS",
                    "pub fn context_child_count",
                    "pub fn context_rule_node",
                    "pub fn context_start",
                    "pub fn context_text",
                    "pub fn r#type",
                    "pub fn self_",
                    "AliasCollisionParser_EOF",
                    "AliasCollisionParser_ID",
                    "__active_context_view_with_attrs::<",
                ]
                .iter()
                .any(|needle| line.contains(needle))
            })
            .map(str::trim)
            .collect::<Vec<_>>()
            .join("\n")
    };
    insta::assert_snapshot!(
        "antlr4rust_compat_generated_surface",
        format!(
            "=== C ===\n{}\n=== Java ===\n{}\n=== Alias only ===\n{}\n=== Alias collision ===\n{}",
            excerpt(&c_parser),
            excerpt(&java_parser),
            excerpt(&alias_parser),
            excerpt(&alias_collision_parser)
        )
    );

    let test_source = r####"
#[cfg(test)]
mod antlr4rust_compat_tests {
    use super::alias_collision_lexer::AliasCollisionLexer;
    use super::alias_collision_parser::AliasCollisionParser;
    use super::alias_only_lexer::AliasOnlyLexer;
    use super::alias_only_parser::AliasOnlyParser;
    use super::c_compat_lexer::CCompatLexer;
    use super::c_compat_parser::CCompatParser;
    use super::java_compat_lexer::JavaCompatLexer;
    use super::java_compat_parser::JavaCompatParser;
    use antlr4_runtime::{CommonTokenStream, InputStream, Parser as _};

    fn c_assignment(input: &str, native: bool) -> (bool, usize) {
        let lexer = CCompatLexer::new(InputStream::new(input));
        let mut parser = CCompatParser::new(CommonTokenStream::new(lexer));
        let parsed = if native {
            parser.native_assignment()
        } else {
            parser.assignment()
        }
        .is_ok();
        (parsed, parser.number_of_syntax_errors())
    }

    #[test]
    fn token_aliases_are_available_without_an_input_or_context_receiver() {
        let lexer = AliasOnlyLexer::new(InputStream::new("name!"));
        let mut parser = AliasOnlyParser::new(CommonTokenStream::new(lexer));
        assert!(parser.start().is_ok());
        assert_eq!(parser.number_of_syntax_errors(), 0);
    }

    #[test]
    fn alias_scopes_and_member_imports_compile_and_execute() {
        let lexer = AliasCollisionLexer::new(InputStream::new("scope"));
        let mut parser = AliasCollisionParser::new(CommonTokenStream::new(lexer));
        assert!(parser.start().is_ok());
        assert_eq!(parser.number_of_syntax_errors(), 0);

        let lexer = AliasCollisionLexer::new(InputStream::new("cross"));
        let mut parser = AliasCollisionParser::new(CommonTokenStream::new(lexer));
        assert!(parser.cross_body().is_ok());
        assert_eq!(parser.number_of_syntax_errors(), 0);
    }

    fn java_not_assign(input: &str, native: bool) -> (bool, usize) {
        let lexer = JavaCompatLexer::new(InputStream::new(input));
        let mut parser = JavaCompatParser::new(CommonTokenStream::new(lexer));
        let parsed = if native {
            parser.native_not_identifier_assign()
        } else {
            parser.not_identifier_assign()
        }
        .is_ok();
        (parsed, parser.number_of_syntax_errors())
    }

    fn java_end_lookahead(input: &str, native: bool) -> (bool, usize) {
        let lexer = JavaCompatLexer::new(InputStream::new(input));
        let mut parser = JavaCompatParser::new(CommonTokenStream::new(lexer));
        let parsed = if native {
            parser.native_end_lookahead()
        } else {
            parser.end_lookahead()
        }
        .is_ok();
        (parsed, parser.number_of_syntax_errors())
    }

    fn java_components(input: &str, native: bool) -> (bool, usize) {
        let lexer = JavaCompatLexer::new(InputStream::new(input));
        let mut parser = JavaCompatParser::new(CommonTokenStream::new(lexer));
        let parsed = if native {
            parser.native_record_component_list()
        } else {
            parser.record_component_list()
        }
        .is_ok();
        (parsed, parser.number_of_syntax_errors())
    }

    #[test]
    fn lookahead_and_missing_token_behavior_match_the_native_stream() {
        for input in ["left=right", "left right"] {
            assert_eq!(
                c_assignment(input, false),
                c_assignment(input, true),
                "legacy C input facade diverged for {input:?}"
            );
        }
        assert_eq!(c_assignment("left=right", false), (true, 0));

        for input in ["name", "module", "name=value"] {
            assert_eq!(
                java_not_assign(input, false),
                java_not_assign(input, true),
                "legacy Java token aliases diverged for {input:?}"
            );
        }
        assert_eq!(java_not_assign("name", false), (true, 0));
        assert_eq!(java_not_assign("module", false), (true, 0));

        for input in ["name", "name=value"] {
            assert_eq!(
                java_end_lookahead(input, false),
                java_end_lookahead(input, true),
                "legacy Java EOF lookahead diverged for {input:?}"
            );
        }
        assert_eq!(java_end_lookahead("name", false), (true, 0));
        let rejected = java_end_lookahead("name=value", false);
        assert!(rejected.1 > 0);
    }

    #[test]
    fn dynamic_lookbehind_and_inline_action_execute() {
        for native in [false, true] {
            let lexer = CCompatLexer::new(InputStream::new("first second"));
            let mut parser = CCompatParser::new(CommonTokenStream::new(lexer));
            let parsed = if native {
                parser.native_history()
            } else {
                parser.history()
            };
            assert!(parsed.is_ok());
            assert_eq!(parser.number_of_syntax_errors(), 0);

            let lexer = CCompatLexer::new(InputStream::new("action"));
            let mut parser = CCompatParser::new(CommonTokenStream::new(lexer));
            let parsed = if native {
                parser.native_inline_action()
            } else {
                parser.inline_action()
            };
            assert!(parsed.is_ok());
            assert_eq!(parser.number_of_syntax_errors(), 0);
        }
    }

    #[test]
    fn active_context_contains_only_children_matched_before_the_predicate() {
        for input in [
            "first,second...",
            "first",
            "first...,second",
            // The syntactically selected ASSIGN branch does not evaluate the
            // predicate belonging to the EOF branch.
            "first...=",
        ] {
            assert_eq!(
                java_components(input, false),
                java_components(input, true),
                "legacy active context diverged for {input:?}"
            );
        }
        assert_eq!(java_components("first,second...", false), (true, 0));
        assert_eq!(java_components("first", false), (true, 0));
        assert_eq!(java_components("first...=", false), (true, 0));
        let rejected = java_components("first...,second", false);
        assert!(rejected.1 > 0);
    }

    #[test]
    fn repeated_tokens_and_common_method_collisions_use_legacy_getters() {
        let lexer = JavaCompatLexer::new(InputStream::new("first second"));
        let mut parser = JavaCompatParser::new(CommonTokenStream::new(lexer));
        assert!(parser.repeated_tokens().is_ok());
        assert_eq!(parser.number_of_syntax_errors(), 0);

        let lexer = JavaCompatLexer::new(InputStream::new(
            "text start child_count rule_node",
        ));
        let mut parser = JavaCompatParser::new(CommonTokenStream::new(lexer));
        assert!(parser.common_accessor_collisions().is_ok());
        assert_eq!(parser.number_of_syntax_errors(), 0);
    }

    #[test]
    fn active_context_reads_attributes_mutated_by_init() {
        let lexer = JavaCompatLexer::new(InputStream::new("value"));
        let mut parser = JavaCompatParser::new(CommonTokenStream::new(lexer));
        assert!(parser.live_attributes().is_ok());
        assert_eq!(parser.number_of_syntax_errors(), 0);

        let lexer = JavaCompatLexer::new(InputStream::new("value"));
        let mut parser = JavaCompatParser::new(CommonTokenStream::new(lexer));
        assert!(parser.same_body_attributes().is_ok());
        assert_eq!(parser.number_of_syntax_errors(), 0);
    }

    #[test]
    fn keyword_compatibility_getter_and_unrelated_context_compile() {
        let lexer = JavaCompatLexer::new(InputStream::new("type self"));
        let mut parser = JavaCompatParser::new(CommonTokenStream::new(lexer));
        assert!(parser.keyword_accessor().is_ok());
        assert_eq!(parser.number_of_syntax_errors(), 0);

        let lexer = JavaCompatLexer::new(InputStream::new("text value"));
        let mut parser = JavaCompatParser::new(CommonTokenStream::new(lexer));
        assert!(parser.unrelated().is_ok());
        assert_eq!(parser.number_of_syntax_errors(), 0);
    }
}
"####;

    assert_generated_project(
        temp.path(),
        &[
            "alias_only_lexer.rs",
            "alias_only_parser.rs",
            "alias_collision_lexer.rs",
            "alias_collision_parser.rs",
            "c_compat_lexer.rs",
            "c_compat_parser.rs",
            "java_compat_lexer.rs",
            "java_compat_parser.rs",
        ],
        test_source,
    );
}

#[allow(clippy::disallowed_methods)] // Snapshot and path normalization are test-only.
#[test]
fn unsupported_antlr4rust_surface_fails_at_its_semantic_coordinate() {
    let temp = temporary_directory("antlr4rust-diagnostics");
    let mut diagnostics = Vec::new();
    let mut cases = [
        (
            "BadRecog",
            r#"{
                let _not_code = "recog.output()";
                // recog.input.unknown(1)
                recog.input.peek(1) == 1
            }"#,
        ),
        (
            "BadContext",
            r#"{
                _localctx.context().is_some()
            }"#,
        ),
        (
            "BadArity",
            r#"{
                recog.input.la(1, 2) == 1
            }"#,
        ),
        (
            "UnclassifiableRust",
            r#"{
                let _broken = (;
                let _: Option<UnclassifiableRustParser_A> = None;
                true
            }"#,
        ),
    ]
    .into_iter()
    .map(|(name, predicate)| {
        (
            name,
            format!("grammar {name};\n\nstart\n    : {predicate}? A EOF\n    ;\n\nA: 'a';\n"),
        )
    })
    .collect::<Vec<_>>();
    cases.extend([
        (
            "BadInit",
            "grammar BadInit;\n\n\
             start\n\
             @init { let _ = recog.input.peek(1); }\n\
                 : A EOF\n\
                 ;\n\n\
             A: 'a';\n"
                .to_owned(),
        ),
        (
            "BadAfter",
            "grammar BadAfter;\n\n\
             start\n\
             @after { let _ = recog.input.peek(1); }\n\
                 : A EOF\n\
                 ;\n\n\
             A: 'a';\n"
                .to_owned(),
        ),
        (
            "BadLexer",
            "lexer grammar BadLexer;\n\n\
             A: 'a' { let _ = recog.input.la(1); };\n"
                .to_owned(),
        ),
        (
            "BadLexerContext",
            "lexer grammar BadLexerContext;\n\n\
             A: 'a' { let _ = _localctx.as_deref(); };\n"
                .to_owned(),
        ),
        (
            "AccessorCollision",
            "grammar AccessorCollision;\n\n\
             start\n\
                 : item+ item_all { _localctx.as_deref().is_some() }? EOF\n\
                 ;\n\
             item: ITEM;\n\
             item_all: ALL;\n\
             ITEM: 'item';\n\
             ALL: 'all';\n\
             WS: [ \\t\\r\\n]+ -> skip;\n"
                .to_owned(),
        ),
        (
            "BadMemberFieldType",
            "grammar BadMemberFieldType;\n\n\
             @parser::members {\n\
                 broken: [u8; BadMemberFieldTypeParser_A +] = [];\n\
             }\n\n\
             start: A EOF;\n\
             A: 'a';\n"
                .to_owned(),
        ),
        (
            "BadMemberFieldInitializer",
            "grammar BadMemberFieldInitializer;\n\n\
             @parser::members {\n\
                 broken: i32 = BadMemberFieldInitializerParser_A +;\n\
             }\n\n\
             start: A EOF;\n\
             A: 'a';\n"
                .to_owned(),
        ),
        (
            "BadMemberImplItem",
            "grammar BadMemberImplItem;\n\n\
             @parser::members {\n\
                 fn broken(&self) -> i32 { BadMemberImplItemParser_A + }\n\
             }\n\n\
             start: A EOF;\n\
             A: 'a';\n"
                .to_owned(),
        ),
        (
            "BadMemberModuleItem",
            "grammar BadMemberModuleItem;\n\n\
             @parser::members {\n\
                 impl Missing {\n\
                     fn broken() -> i32 { BadMemberModuleItemParser_A + }\n\
                 }\n\
             }\n\n\
             start: A EOF;\n\
             A: 'a';\n"
                .to_owned(),
        ),
    ]);
    for (name, grammar_source) in cases {
        let grammar = temp.path().join(format!("{name}.g4"));
        fs::write(&grammar, grammar_source).expect("diagnostic fixture should be writable");
        let out = temp.path().join(format!("generated-{name}"));
        let output = run_antlr4_rust_gen(&[
            grammar.as_os_str(),
            OsStr::new("--actions"),
            OsStr::new("embedded"),
            OsStr::new("--out-dir"),
            out.as_os_str(),
        ]);
        assert!(
            !output.status.success(),
            "unsupported compatibility shape unexpectedly generated: {}",
            utf8(&output.stdout)
        );
        let path = grammar
            .to_str()
            .expect("temporary grammar path should be UTF-8");
        let root = temp
            .path()
            .to_str()
            .expect("temporary directory path should be UTF-8");
        let stderr = replace_miette_path(utf8(&output.stderr), path, "$GRAMMAR");
        diagnostics.push(replace_miette_path(&stderr, root, "$TMP"));
    }
    insta::assert_snapshot!(
        "unsupported_antlr4rust_surface_diagnostics",
        diagnostics.join("\n")
    );
}

#[test]
fn imported_predicate_manifest_uses_its_structural_source_owner() {
    let temp = temporary_directory("imported-predicate");
    let root = temp.path().join("Root.g4");
    let delegate = temp.path().join("Delegate.g4");
    let tokens = temp.path().join("Tokens.g4");
    let out = temp.path().join("generated");
    fs::write(
        &root,
        "parser grammar Root;\n\
         import Delegate;\n\
         options { tokenVocab=Tokens; }\n\
         start: delegated EOF;\n",
    )
    .expect("root grammar should be writable");
    fs::write(
        &delegate,
        "parser grammar Delegate;\n\
         delegated: {featureEnabled()}? ID;\n",
    )
    .expect("delegate grammar should be writable");
    fs::write(
        &tokens,
        "lexer grammar Tokens;\n\
         ID: [a-z]+;\n\
         WS: [ \\t\\r\\n]+ -> skip;\n",
    )
    .expect("token grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        root.as_os_str(),
        tokens.as_os_str(),
        OsStr::new("-I"),
        temp.path().as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let manifest =
        fs::read_to_string(out.join("semantics.json")).expect("manifest should be emitted");
    assert!(manifest.contains("\"name\": \"Root\""), "{manifest}");
    assert!(
        manifest.contains("\"body\": \"featureEnabled()\""),
        "{manifest}"
    );
    assert!(manifest.contains("\"line\": 2"), "{manifest}");
}

#[test]
fn imported_parser_predicate_generates_typed_hook_from_structural_body() {
    let temp = temporary_directory("imported-parser-hook");
    let root = temp.path().join("Root.g4");
    let delegate = temp.path().join("Delegate.g4");
    let tokens = temp.path().join("Tokens.g4");
    let out = temp.path().join("generated");
    fs::write(
        &root,
        "parser grammar Root;\n\
         import Delegate;\n\
         options { tokenVocab=Tokens; }\n\
         start: delegated EOF;\n",
    )
    .expect("root grammar should be writable");
    fs::write(
        &delegate,
        "parser grammar Delegate;\ndelegated: {isTypeName()}? ID;\n",
    )
    .expect("delegate grammar should be writable");
    fs::write(&tokens, "lexer grammar Tokens;\nID: [a-z]+;\n")
        .expect("token grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        root.as_os_str(),
        tokens.as_os_str(),
        OsStr::new("-I"),
        temp.path().as_os_str(),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let parser = fs::read_to_string(out.join("root.rs")).expect("parser should be emitted");
    assert!(parser.contains("pub trait RootHooks"), "{parser}");
    assert!(parser.contains("fn is_type_name"), "{parser}");
    assert!(
        parser.contains("(1, 0) => Some(self.0.is_type_name(ctx))"),
        "{parser}"
    );
}

#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
#[test]
fn imported_antlr4rust_alias_uses_the_action_source_owner() {
    let temp = temporary_directory("imported-antlr4rust-alias");
    let root = temp.path().join("Root.g4");
    let delegate = temp.path().join("Delegate.g4");
    let out = temp.path().join("generated");
    fs::write(
        &root,
        "grammar Root;\n\
         import Delegate;\n\
         start: delegated EOF;\n\
         WS: [ \\t\\r\\n]+ -> skip;\n",
    )
    .expect("root grammar should be writable");
    fs::write(
        &delegate,
        "grammar Delegate;\n\
         @parser::members {\n\
             marker: i32 = DelegateParser_ID;\n\
             fn imported_member_alias_matches(&self) -> bool {\n\
                 DelegateParser_ID == Self::ID && self.marker == Self::ID\n\
             }\n\
         }\n\
         delegated: {\n\
             DelegateParser_ID == ID && self.imported_member_alias_matches()\n\
         }? ID;\n\
         ID: [a-z]+;\n",
    )
    .expect("delegate grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        root.as_os_str(),
        OsStr::new("-I"),
        temp.path().as_os_str(),
        OsStr::new("--actions"),
        OsStr::new("embedded"),
        OsStr::new("--sem-unknown"),
        OsStr::new("error"),
        OsStr::new("--require-full-semantics"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let parser = fs::read_to_string(out.join("root_parser.rs")).expect("parser should be emitted");
    let alias_excerpt = parser
        .lines()
        .filter(|line| line.contains("Parser_ID"))
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n");
    insta::assert_snapshot!("imported_antlr4rust_alias_owner", alias_excerpt);
    assert!(
        parser.contains("marker: __antlr4rust_token_aliases::DelegateParser_ID")
            && parser.contains("__antlr4rust_token_aliases::DelegateParser_ID == Self::ID")
            && parser.contains("self.marker == Self::ID"),
        "imported member fields and methods must use their source grammar's alias owner"
    );
    assert_generated_modules_compile(temp.path(), &["root_lexer.rs", "root_parser.rs"]);
}

#[test]
fn imported_antlr4rust_implicit_alias_uses_its_source_literal() {
    let temp = temporary_directory("imported-antlr4rust-implicit-alias");
    let root = temp.path().join("Root.g4");
    let delegate = temp.path().join("Delegate.g4");
    let out = temp.path().join("generated");
    fs::write(
        &root,
        "grammar Root;\n\
         import Delegate;\n\
         start: 'r' {\n\
             recog.input.la(1) == RootParser_T__1\n\
         }? delegated EOF;\n",
    )
    .expect("root grammar should be writable");
    fs::write(
        &delegate,
        "grammar Delegate;\n\
         delegated: {\n\
             recog.input.la(1) == DelegateParser_T__0\n\
         }? 'd';\n",
    )
    .expect("delegate grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        root.as_os_str(),
        OsStr::new("-I"),
        temp.path().as_os_str(),
        OsStr::new("--actions"),
        OsStr::new("embedded"),
        OsStr::new("--sem-unknown"),
        OsStr::new("error"),
        OsStr::new("--require-full-semantics"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let parser = fs::read_to_string(out.join("root_parser.rs")).expect("parser should be emitted");
    assert!(
        parser.contains("const DelegateParser_T__0: i32 = 2;"),
        "the delegate's local T__0 must map to its 'd' literal, not root 'r'\n{}",
        matching_lines(&parser, "DelegateParser_T__0")
    );
    assert!(
        parser.contains("const RootParser_T__1: i32 = 2;"),
        "the root owner must retain merged implicit-token numbering\n{}",
        matching_lines(&parser, "RootParser_T__1")
    );
    assert_generated_project(
        temp.path(),
        &["root_lexer.rs", "root_parser.rs"],
        r#"
#[cfg(test)]
mod imported_implicit_alias_tests {
    use super::root_lexer::RootLexer;
    use super::root_parser::RootParser;
    use antlr4_runtime::{CommonTokenStream, InputStream, Parser as _};

    #[test]
    fn delegate_predicate_matches_its_local_literal() {
        let lexer = RootLexer::new(InputStream::new("rd"));
        let mut parser = RootParser::new(CommonTokenStream::new(lexer));
        assert!(parser.start().is_ok());
        assert_eq!(parser.number_of_syntax_errors(), 0);
    }
}
"#,
    );
}

#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
#[test]
fn antlr4rust_alias_lowering_preserves_type_positions() {
    let temp = temporary_directory("antlr4rust-alias-type-positions");
    let grammar = temp.path().join("TypePosition.g4");
    let out = temp.path().join("generated");
    fs::write(
        &grammar,
        "grammar TypePosition;\n\
         @parser::members {\n\
             struct TypePositionParser_ID {\n\
                 marker: i32,\n\
             }\n\
             fn preserve_generic<TypePositionParser_ID>(\n\
                 value: Option<TypePositionParser_ID>,\n\
             ) -> Option<TypePositionParser_ID> {\n\
                 value\n\
             }\n\
         }\n\
         start: {\n\
             let _unicode = \"\u{e9}\"; let _: Option<TypePositionParser_ID> =\n\
                 Self::preserve_generic::<TypePositionParser_ID>(None);\n\
             TypePositionParser_ID == ID\n\
         }? ID EOF;\n\
         ID: [a-z]+;\n\
         WS: [ \\t\\r\\n]+ -> skip;\n",
    )
    .expect("type-position grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--actions"),
        OsStr::new("embedded"),
        OsStr::new("--sem-unknown"),
        OsStr::new("error"),
        OsStr::new("--require-full-semantics"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let parser =
        fs::read_to_string(out.join("type_position_parser.rs")).expect("parser should be emitted");
    let alias_excerpt = parser
        .lines()
        .filter(|line| line.contains("TypePositionParser_ID"))
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n");
    assert_generated_modules_compile(
        temp.path(),
        &["type_position_lexer.rs", "type_position_parser.rs"],
    );
    insta::assert_snapshot!("antlr4rust_alias_type_positions", alias_excerpt);
}

#[allow(clippy::disallowed_methods)] // `insta` assertion macros unwrap internal I/O.
#[test]
fn antlr4rust_alias_owner_preserves_source_grammar_spelling() {
    let temp = temporary_directory("antlr4rust-alias-owner-spelling");
    let grammar = temp.path().join("XML.g4");
    let out = temp.path().join("generated");
    fs::write(
        &grammar,
        "grammar XML;\n\
         start: { XMLParser_ID == ID }? ID EOF;\n\
         ID: [a-z]+;\n\
         WS: [ \\t\\r\\n]+ -> skip;\n",
    )
    .expect("acronym grammar should be writable");

    let output = run_antlr4_rust_gen(&[
        grammar.as_os_str(),
        OsStr::new("--actions"),
        OsStr::new("embedded"),
        OsStr::new("--sem-unknown"),
        OsStr::new("error"),
        OsStr::new("--require-full-semantics"),
        OsStr::new("--out-dir"),
        out.as_os_str(),
    ]);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        utf8(&output.stdout),
        utf8(&output.stderr)
    );
    let parser = fs::read_to_string(out.join("xml_parser.rs")).expect("parser should be emitted");
    let alias_excerpt = parser
        .lines()
        .filter(|line| line.contains("Parser_ID"))
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n");
    assert_generated_modules_compile(temp.path(), &["xml_lexer.rs", "xml_parser.rs"]);
    insta::assert_snapshot!("antlr4rust_alias_owner_spelling", alias_excerpt);
}
