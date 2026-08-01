// A user-authored module item wins over a metadata-shaped compatibility alias.
grammar AliasCollision;

@parser::members {
    marker: i32 = AliasCollisionParser_FIELD_INIT;
    field_type: [u8; AliasCollisionParser_FIELD_TYPE as usize] =
        [0; AliasCollisionParser_FIELD_TYPE as usize];
    #[cfg(any())]
    conditional_field: i32 = AliasCollisionParser_FIELD_INIT;
    #[deprecated(note = "member field declaration only")]
    deprecated_field: i32 = 0;

    struct AliasCollisionParser_ID;
    struct AliasCollisionParser_NAMED {
        marker: i32,
    }
    struct __antlr4rust_token_aliases;
    struct __Antlr4RustContext;
    struct __Antlr4RustInput;
    struct __Antlr4RustTokenView;
    struct ConstGenericMember<const N: usize>;
    struct ConstExpression<const N: usize>;
    struct AssociatedConstMember;
    impl AssociatedConstMember {
        const AliasCollisionParser_ASSOCIATED_CONST: i32 = 0;

        fn token_alias() -> i32 {
            AliasCollisionParser_ASSOCIATED_CONST
        }
    }
    impl<const AliasCollisionParser_IMPL_CONST: usize>
        ConstGenericMember<AliasCollisionParser_IMPL_CONST>
    {
        fn value() -> usize {
            AliasCollisionParser_IMPL_CONST
        }
    }
    use antlr4_runtime::TOKEN_EOF as AliasCollisionParser_EOF;
    use self::{AliasCollisionParser_MODULE as RenamedModule};
    use self::{AliasCollisionParser_MEMBER_ONLY as RenamedMemberOnly};
    use self::AliasCollisionParser_DIRECT;
    use ::{std::fmt};
    #[cfg(
        any()
    )]
    use antlr4_runtime::TOKEN_EOF as AliasCollisionParser_CFG;
    #[cfg_attr(all(), cfg(any()))]
    use antlr4_runtime::TOKEN_EOF as AliasCollisionParser_CFG_ATTR;

    fn member_alias_matches(&self) -> bool {
        AliasCollisionParser_MODULE == Self::MODULE
    }

    fn AliasCollisionParser_METHOD_NAME(&self) -> bool {
        AliasCollisionParser_METHOD_NAME == Self::METHOD_NAME
    }

    fn block_tail_alias(&self) -> i32 {
        AliasCollisionParser_MODULE
    }

    fn conditional_block_tail_alias(&self) -> i32 {
        if self.marker == Self::FIELD_INIT {
            AliasCollisionParser_MODULE
        } else {
            AliasCollisionParser_SCOPE
        }
    }

    fn closure_block_tail_alias(&self) -> i32 {
        (|_| -> i32 { AliasCollisionParser_MODULE })(())
    }

    struct MemberHelper;

    impl MemberHelper {
        fn module_alias_matches() -> bool {
            use std::fmt::Write as _;
            let mut rendered = String::new();
            let _ = write!(&mut rendered, "module");
            !rendered.is_empty() && AliasCollisionParser_MODULE == MODULE
        }
    }
}

start
    : {
        let _user_symbol = AliasCollisionParser_ID;
        let _user_import = AliasCollisionParser_EOF;
        let _compat_alias = AliasCollisionParser_MODULE;
        let _renamed_import = RenamedModule;
        let _member_only_import = RenamedMemberOnly;
        let _conditional_alias = AliasCollisionParser_CFG;
        let cfg_attr_alias_ok = AliasCollisionParser_CFG_ATTR == CFG_ATTR;
        #[cfg(any())]
        use antlr4_runtime::TOKEN_EOF as AliasCollisionParser_ACTION_CFG;
        let action_cfg_alias_ok =
            AliasCollisionParser_ACTION_CFG == ACTION_CFG;
        let inline_const_alias_ok =
            const { AliasCollisionParser_CONST_BLOCK } == CONST_BLOCK;
        let associated_const_alias_ok =
            AssociatedConstMember::token_alias() == ASSOCIATED_CONST;
        let raw_macro_ok =
            stringify!(AliasCollisionParser_MACRO) == "AliasCollisionParser_MACRO";
        macro_rules! assert {
            (AliasCollisionParser_SHADOWED_MACRO) => {
                true
            };
        }
        let shadowed_macro_ok = assert!(AliasCollisionParser_SHADOWED_MACRO);
        #[allow(unexpected_cfgs)]
        #[cfg(AliasCollisionParser_ATTRIBUTE)]
        let _attribute_token_tree = ();
        let πrecog = 41;
        let πAliasCollisionParser_UNICODE = 42;
        let unicode_identifiers_ok =
            πrecog + 1 == πAliasCollisionParser_UNICODE;
        std::thread_local! {
            static AliasCollisionParser_MACRO_IDENT: std::cell::Cell<bool> =
                const { std::cell::Cell::new(true) };
        }
        let macro_ident_ok = true;
        let input_facade_ok = recog.input.la(1) > 0
            && recog
                .input
                .lt(1)
                .map(|token| !token.get_text().is_empty())
                .unwrap_or(false);
        let before_scope = AliasCollisionParser_SCOPE;
        {
            let AliasCollisionParser_SCOPE = 99;
            assert_eq!(AliasCollisionParser_SCOPE, 99);
        }
        let after_scope = AliasCollisionParser_SCOPE;
        let AliasCollisionParser_CROSS = 7;
        let _cross_body_local = AliasCollisionParser_CROSS;
        let AliasCollisionParser_LOCAL = 7;
        let _local_binding = AliasCollisionParser_LOCAL;
        struct ScopeInput {
            value: Option<i32>,
            values: [i32; 1],
        }
        let if_binding_ok = if let Some(AliasCollisionParser_IF) =
            (ScopeInput { value: Some(9), values: [10] }).value
        {
            AliasCollisionParser_IF == 9
        } else {
            false
        };
        let mut for_binding_ok = false;
        for AliasCollisionParser_FOR in
            (ScopeInput { value: None, values: [10] }).values
        {
            for_binding_ok = AliasCollisionParser_FOR == 10;
        }
        let match_binding_ok = match Some(7) {
            Some(AliasCollisionParser_MATCH @ _) => AliasCollisionParser_MATCH == 7,
            None => false,
        };
        let leading_match_binding_ok = match Some(8) {
            | Some(AliasCollisionParser_ARM @ _) => AliasCollisionParser_ARM == 8,
            None => false,
        };
        let block_match_binding_ok = match Some(13) {
            None => { false }
            Some(AliasCollisionParser_ARM @ _) => {
                AliasCollisionParser_ARM == 13
            }
        };
        let not_equal_alias_ok = AliasCollisionParser_MODULE != 0;
        let if_head_alias_ok = if MODULE == AliasCollisionParser_MODULE {
            true
        } else {
            false
        };
        let mut while_head_alias_ok = false;
        while MODULE == AliasCollisionParser_MODULE {
            while_head_alias_ok = true;
            break;
        }
        let match_head_alias_ok = match AliasCollisionParser_MODULE {
            value if value == MODULE => true,
            _ => false,
        };
        let let_chain_binding_ok = if let Some(AliasCollisionParser_CHAIN) = Some(12)
            && AliasCollisionParser_CHAIN == 12
        {
            true
        } else {
            false
        };
        let leading_or_alias_ok = matches!(
            MODULE,
            | AliasCollisionParser_MODULE | AliasCollisionParser_SCOPE
        );
        let matches_binding_ok = matches!(
            Some(Self::MATCHES_BINDING),
            AliasCollisionParser_MATCHES_BINDING @ Some(_)
                if AliasCollisionParser_MATCHES_BINDING
                    == Some(Self::MATCHES_BINDING)
        );
        let nested_closure_binding_ok =
            (|_outer| |AliasCollisionParser_PARAM: i32| {
                AliasCollisionParser_PARAM
            })(1)(2) == 2;
        let async_closure =
            async |AliasCollisionParser_ASYNC: usize| {
                AliasCollisionParser_ASYNC
            };
        fn require_usize_future<F: std::future::Future<Output = usize>>(_: F) {}
        require_usize_future(async_closure(23));
        fn const_generic_value<const AliasCollisionParser_CONST_GENERIC: usize>() -> usize {
            AliasCollisionParser_CONST_GENERIC
        }
        let const_generic_ok = const_generic_value::<17>() == 17;
        fn precise_capture<T>() -> impl Copy + use<T> {
            AliasCollisionParser_PRECISE_CAPTURE
        }
        let _precise_capture = precise_capture::<u8>();
        let impl_const_generic_ok = ConstGenericMember::<19>::value() == 19;
        let _: ConstExpression<{
            AliasCollisionParser_CONST_EXPRESSION as usize
        }> = ConstExpression;
        fn braced_return(
            AliasCollisionParser_BRACED_PARAM: i32,
        ) -> [i32; { 1 }] {
            [AliasCollisionParser_BRACED_PARAM]
        }
        let braced_parameter_ok = braced_return(31) == [31];
        enum LocalEnum {
            AliasCollisionParser_ENUM,
        }
        let enum_variant_ok = matches!(
            LocalEnum::AliasCollisionParser_ENUM,
            LocalEnum::AliasCollisionParser_ENUM
        );
        let named_struct = AliasCollisionParser_NAMED { marker: 14 };
        struct AliasCollisionParser_LOCAL_TYPE {
            marker: i32,
        }
        let local_type = AliasCollisionParser_LOCAL_TYPE { marker: 15 };
        struct AliasFields {
            AliasCollisionParser_FIELD: i32,
        }
        let explicit = AliasFields {
            AliasCollisionParser_FIELD: AliasCollisionParser_FIELD,
        };
        let shorthand = AliasFields {
            AliasCollisionParser_FIELD,
        };
        fn apply<F: Fn(i32) -> i32>(
            AliasCollisionParser_PARAM: i32,
            function: F,
        ) -> i32 {
            function(AliasCollisionParser_PARAM)
        }
        before_scope == SCOPE
            && after_scope == SCOPE
            && if_binding_ok
            && for_binding_ok
            && match_binding_ok
            && leading_match_binding_ok
            && block_match_binding_ok
            && not_equal_alias_ok
            && if_head_alias_ok
            && while_head_alias_ok
            && match_head_alias_ok
            && let_chain_binding_ok
            && cfg_attr_alias_ok
            && action_cfg_alias_ok
            && inline_const_alias_ok
            && associated_const_alias_ok
            && raw_macro_ok
            && shadowed_macro_ok
            && macro_ident_ok
            && unicode_identifiers_ok
            && input_facade_ok
            && leading_or_alias_ok
            && matches_binding_ok
            && nested_closure_binding_ok
            && const_generic_ok
            && impl_const_generic_ok
            && braced_parameter_ok
            && enum_variant_ok
            && named_struct.marker == 14
            && local_type.marker == 15
            && AliasCollisionParser_LOCAL_TYPE == LOCAL_TYPE
            && AliasCollisionParser_NAMED == NAMED
            && self.marker == Self::FIELD_INIT
            && self.field_type.len() == Self::FIELD_TYPE as usize
            && explicit.AliasCollisionParser_FIELD == FIELD
            && shorthand.AliasCollisionParser_FIELD == FIELD
            && apply(11, |value| value) == 11
            && AliasCollisionParser_DIRECT == DIRECT
            && self.member_alias_matches()
            && self.AliasCollisionParser_METHOD_NAME()
            && self.block_tail_alias() == Self::MODULE
            && self.conditional_block_tail_alias() == Self::MODULE
            && self.closure_block_tail_alias() == Self::MODULE
            && MemberHelper::module_alias_matches()
            && _localctx
                .as_deref()
                .map(|ctx| ctx.ID().is_none())
                .unwrap_or(false)
    }? (
        ID
        | MODULE
        | MEMBER_ONLY
        | DIRECT
        | CFG
        | CFG_ATTR
        | MACRO
        | MACRO_IDENT
        | ENUM
        | CONST_GENERIC
        | IMPL_CONST
        | FIELD_TYPE
        | BRACED_PARAM
        | ASYNC
        | SCOPE
        | CROSS
        | LOCAL
        | MATCH
        | ARM
        | CHAIN
        | NAMED
        | FIELD_INIT
        | FIELD
        | IF
        | FOR
        | PARAM
        | LOCAL_TYPE
        | METHOD_NAME
        | CONST_EXPRESSION
        | SHADOWED_MACRO
        | ATTRIBUTE
        | UNICODE
        | ACTION_CFG
        | CONST_BLOCK
        | ASSOCIATED_CONST
        | MATCHES_BINDING
        | PRECISE_CAPTURE
    ) EOF
    ;

crossBody
    : { AliasCollisionParser_CROSS == CROSS }? CROSS EOF
    ;

MODULE: 'module';
MEMBER_ONLY: 'member';
DIRECT: 'direct';
CFG: 'cfg';
CFG_ATTR: 'cfg_attr';
MACRO: 'macro';
MACRO_IDENT: 'macro_ident';
ENUM: 'enum';
CONST_GENERIC: 'const_generic';
IMPL_CONST: 'impl_const';
FIELD_TYPE: 'field_type';
BRACED_PARAM: 'braced_param';
ASYNC: 'async';
SCOPE: 'scope';
CROSS: 'cross';
LOCAL: 'local';
MATCH: 'match';
ARM: 'arm';
CHAIN: 'chain';
NAMED: 'named';
FIELD_INIT: 'field_init';
FIELD: 'field';
IF: 'if';
FOR: 'for';
PARAM: 'param';
LOCAL_TYPE: 'local_type';
METHOD_NAME: 'method_name';
CONST_EXPRESSION: 'const_expression';
SHADOWED_MACRO: 'shadowed_macro';
ATTRIBUTE: 'attribute';
UNICODE: 'unicode';
ACTION_CFG: 'action_cfg';
CONST_BLOCK: 'const_block';
ASSOCIATED_CONST: 'associated_const';
MATCHES_BINDING: 'matches_binding';
PRECISE_CAPTURE: 'precise_capture';
ID: [a-z]+;
WS: [ \t\r\n]+ -> skip;
