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
    struct r#__Antlr4RustInput;
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
    use std::fmt::Result as AliasCollisionParser_TYPE_ONLY;
    use antlr4_runtime::TOKEN_EOF as AliasCollisionParser_VALUE_IMPORT;
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
        let type_only_import_alias_ok =
            AliasCollisionParser_TYPE_ONLY == Self::TYPE_ONLY;
        let value_import_alias_ok =
            AliasCollisionParser_VALUE_IMPORT == antlr4_runtime::TOKEN_EOF;
        let cfg_attr_alias_ok = AliasCollisionParser_CFG_ATTR == CFG_ATTR;
        #[cfg(any())]
        use antlr4_runtime::TOKEN_EOF as AliasCollisionParser_ACTION_CFG;
        let action_cfg_alias_ok =
            AliasCollisionParser_ACTION_CFG == ACTION_CFG;
        let inline_const_alias_ok =
            const { AliasCollisionParser_CONST_BLOCK } == CONST_BLOCK;
        let associated_const_alias_ok =
            AssociatedConstMember::token_alias() == ASSOCIATED_CONST;
        let format_capture_ok =
            format!("{AliasCollisionParser_FORMAT_CAPTURE}")
                == Self::FORMAT_CAPTURE.to_string();
        let standard_format_capture_ok =
            std::format!("{AliasCollisionParser_STANDARD_FORMAT}")
                == Self::STANDARD_FORMAT.to_string();
        let cfg_disabled_format_ok = {
            #[cfg(any())]
            use missing::format;
            format!("{AliasCollisionParser_CFG_FORMAT}")
                == Self::CFG_FORMAT.to_string()
        };
        let AliasCollisionParser_FORMAT_LOCAL = "local";
        let format_local_ok =
            format!("{AliasCollisionParser_FORMAT_LOCAL}") == "local";
        let c_strings_ok =
            c"value".to_bytes() == b"value"
                && cr#"raw value"#.to_bytes() == b"raw value"
                && c"\xE6".to_bytes() == b"\xE6"
                && c"\u{0_0E6}".to_bytes() == "\u{00E6}".as_bytes();
        unsafe extern "C" {}
        let unsafe_extern_alias_ok =
            AliasCollisionParser_UNSAFE_EXTERN == Self::UNSAFE_EXTERN;
        unsafe extern "C" {
            safe fn safe_foreign();
            safe static SAFE_FOREIGN: i32;
        }
        let safe_foreign_alias_ok =
            AliasCollisionParser_SAFE_FOREIGN == Self::SAFE_FOREIGN;
        fn raw_lifetime<'r#type>(
            value: &'r#type i32,
        ) -> &'r#type i32 {
            let _alias = AliasCollisionParser_RAW_LIFETIME;
            value
        }
        let raw_lifetime_value = 37;
        let raw_lifetime_ok =
            *raw_lifetime(&raw_lifetime_value) == raw_lifetime_value;
        let placeholder_lifetime: Option<&'_ i32> = None;
        let placeholder_lifetime_ok =
            placeholder_lifetime.is_none();
        let raw_reference_value = 43;
        let raw_reference = & raw const raw_reference_value;
        let raw_reference_ok = !raw_reference.is_null();
        let nested_raw_reference_ok =
            unsafe { *&raw const raw_reference_value }
                == raw_reference_value;
        let exponent_underscore_ok = 1e_1 == 10.0;
        let one_sided_range_patterns_ok = match 2 {
            ..=1 => false,
            2.. => true,
        };
        fn attributed_binder(
            _: for<#[cfg(all())] 'a> fn(&'a i32),
        ) {}
        let attributed_binder_ok = true;
        struct DefaultedConst<const N: usize = 3>;
        let _: DefaultedConst = DefaultedConst;
        let const_generic_default_ok = true;
        fn safe(safe: i32) -> i32 {
            safe
        }
        let safe_identifier_ok = safe(29) == 29;
        let unicode_escape_underscores_ok =
            "\u{00_E6}" == "\u{00E6}";
        #[cfg(any())]
        fn multiple_match_inner_attrs() {
            let _ = match 1 {
                #![allow(unused)]
                #![allow(dead_code)]
                1 => true,
                _ => false,
            };
        }
        let multiple_match_inner_attrs_ok = true;
        #[allow(non_snake_case, uncommon_codepoints)]
        let 𞤀 = 47;
        let non_bmp_identifier_ok = 𞤀 == 47;
        let mut underscore_assignment_value = 0;
        _ = {
            underscore_assignment_value = 1;
            99
        };
        let underscore_assignment_ok = underscore_assignment_value == 1;
        macro_rules! receiver_name {
            ($i:ident) => {
                stringify!($i)
            };
        }
        let opaque_receiver_tokens_ok =
            stringify!(recog) == "recog"
                && receiver_name!(_localctx) == "_localctx";
        #[cfg_attr(any(), allow(recog, _localctx))]
        let opaque_attribute_receiver_tokens_ok = true;
        let raw_macro_ok =
            stringify!(AliasCollisionParser_MACRO) == "AliasCollisionParser_MACRO";
        macro_rules! r#match {
            ($i:ident) => {
                stringify!($i) == "AliasCollisionParser_RAW_MACRO_NAME"
            };
        }
        let raw_macro_name_ok =
            r#match!(AliasCollisionParser_RAW_MACRO_NAME);
        macro_rules! raw_string_match {
            ($i:ident) => {{
                let _ = r#""""#;
                stringify!($i) == "AliasCollisionParser_RAW_STRING_MACRO"
            }};
        }
        let raw_string_macro_ok =
            raw_string_match!(AliasCollisionParser_RAW_STRING_MACRO);
        macro_rules! λ {
            ($i:ident) => {
                stringify!($i) == "AliasCollisionParser_UNICODE_MACRO_NAME"
            };
        }
        let unicode_macro_name_ok =
            λ!(AliasCollisionParser_UNICODE_MACRO_NAME);
        macro_rules! alias_value {
            ($i:ident) => {
                $i
            };
        }
        let opaque_macro_alias_ok =
            alias_value!(AliasCollisionParser_OPAQUE_MACRO)
                == Self::OPAQUE_MACRO;
        macro_rules /* keyword */ ! /* bang */ commented_alias /* name */ {
            ($i:ident) => {
                $i
            };
        }
        let commented_macro_header_ok =
            commented_alias!(AliasCollisionParser_OPAQUE_MACRO)
                == Self::OPAQUE_MACRO;
        macro_rules! define_module_alias {
            ($i:ident) => {
                pub(super) fn value() -> i32 {
                    $i
                }
            };
        }
        mod item_macro_module {
            define_module_alias!(AliasCollisionParser_OPAQUE_MACRO);
        }
        let module_item_macro_ok =
            item_macro_module::value() == Self::OPAQUE_MACRO;
        macro_rules! define_impl_alias {
            ($i:ident) => {
                fn value() -> i32 {
                    $i
                }
            };
        }
        struct ItemMacro;
        impl ItemMacro {
            define_impl_alias!(AliasCollisionParser_OPAQUE_MACRO);
        }
        let impl_item_macro_ok =
            ItemMacro::value() == Self::OPAQUE_MACRO;
        macro_rules! assert {
            (AliasCollisionParser_SHADOWED_MACRO) => {
                true
            };
        }
        let shadowed_macro_ok = assert!(AliasCollisionParser_SHADOWED_MACRO);
        mod my_macros {
            macro_rules! assert {
                (AliasCollisionParser_QUALIFIED_MACRO) => {
                    true
                };
            }
            macro_rules! matches {
                ($($tokens:tt)*) => {
                    true
                };
            }
            macro_rules! imported_assert_eq {
                (AliasCollisionParser_IMPORTED_MACRO) => {
                    true
                };
            }
            pub(crate) use assert;
            pub(crate) use imported_assert_eq;
            pub(crate) use matches;
        }
        let qualified_macro_ok =
            my_macros::assert!(AliasCollisionParser_QUALIFIED_MACRO);
        let imported_macro_ok = {
            use my_macros::imported_assert_eq as assert_eq;
            assert_eq!(AliasCollisionParser_IMPORTED_MACRO)
        };
        let standard_qualified_macro_ok =
            std::matches!(Self::MODULE, AliasCollisionParser_MODULE);
        let custom_matches_macro_ok =
            my_macros::matches!(
                Self::MODULE,
                AliasCollisionParser_CUSTOM_MATCHES => fallback
            );
        macro_rules! alias_type {
            ($i:ident) => {
                [u8; $i as usize]
            };
        }
        let opaque_type_value:
            alias_type!(AliasCollisionParser_TYPE_MACRO) =
                [0; Self::TYPE_MACRO as usize];
        let opaque_type_macro_ok =
            opaque_type_value.len() == Self::TYPE_MACRO as usize;
        macro_rules! alias_pattern {
            ($i:ident) => {
                $i
            };
        }
        let opaque_pattern_macro_ok =
            if let alias_pattern!(AliasCollisionParser_PATTERN_MACRO) =
                Self::PATTERN_MACRO
            {
                true
            } else {
                false
            };
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
        let turbofish_match_binding_ok = match Some(16) {
            Some(AliasCollisionParser_TURBOFISH @ _) =>
                Ok::<i32, ()>(AliasCollisionParser_TURBOFISH).unwrap() == 16,
            None => false,
        };
        let closure_match_binding_ok = match 1 {
            AliasCollisionParser_CLOSURE_MATCH @ _ =>
                (move |x, y| AliasCollisionParser_CLOSURE_MATCH + x + y)(2, 3)
                    == 6,
        };
        #[cfg(all())]
        use antlr4_runtime::DEFAULT_CHANNEL
            as AliasCollisionParser_ACTIVE_CFG_USE;
        let active_cfg_use_ok =
            AliasCollisionParser_ACTIVE_CFG_USE
                == antlr4_runtime::DEFAULT_CHANNEL;
        #[cfg(any())]
        use antlr4_runtime::DEFAULT_CHANNEL
            as AliasCollisionParser_INACTIVE_CFG_USE;
        let inactive_cfg_use_ok =
            AliasCollisionParser_INACTIVE_CFG_USE
                == Self::INACTIVE_CFG_USE;
        #[cfg(all())]
        let AliasCollisionParser_ACTIVE_CFG_LET = 73;
        let active_cfg_let_ok =
            AliasCollisionParser_ACTIVE_CFG_LET == 73;
        #[cfg(any())]
        let AliasCollisionParser_INACTIVE_CFG_LET = 74;
        let inactive_cfg_let_ok =
            AliasCollisionParser_INACTIVE_CFG_LET
                == Self::INACTIVE_CFG_LET;
        #[cfg(any())]
        let AliasCollisionParser_DUPLICATE_CFG = 75;
        #[cfg(any())]
        use antlr4_runtime::DEFAULT_CHANNEL
            as AliasCollisionParser_DUPLICATE_CFG;
        let duplicate_cfg_ok =
            AliasCollisionParser_DUPLICATE_CFG
                == Self::DUPLICATE_CFG;
        #[cfg(any())]
        let AliasCollisionParser_STAGED_CFG = 76;
        let staged_cfg_before =
            AliasCollisionParser_STAGED_CFG == Self::STAGED_CFG;
        #[cfg(all())]
        let AliasCollisionParser_STAGED_CFG = 77;
        let staged_cfg_after = AliasCollisionParser_STAGED_CFG == 77;
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
        let if_let_constant_ok =
            if let AliasCollisionParser_MODULE = MODULE {
                true
            } else {
                false
            };
        let mut while_let_constant_ok = false;
        while let AliasCollisionParser_MODULE = MODULE {
            while_let_constant_ok = true;
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
        let nonleading_let_chain_ok =
            if true && let Some(value) = Some(12) {
                value == 12
            } else {
                false
            };
        let const_block_binding_ok =
            if let Some(AliasCollisionParser_CONST_CHAIN @ _) =
                const { Some(24) }
            {
                AliasCollisionParser_CONST_CHAIN == 24
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
        fn accepts_associated_type_bound(
            _: impl Iterator<Item: Copy>,
        ) {}
        let associated_type_bound_ok =
            AliasCollisionParser_ASSOCIATED_BOUND
                == Self::ASSOCIATED_BOUND;
        let pair = ((1, 2), 3);
        let tuple_field_ok = pair.0.1 == 2;
        mod self_visible {
            pub(self) fn helper() -> i32 {
                5
            }

            pub fn value() -> i32 {
                helper()
            }
        }
        let pub_self_ok = self_visible::value() == 5;
        mod self_restricted {
            pub(in self) fn helper() -> i32 {
                6
            }

            pub fn value() -> i32 {
                helper()
            }
        }
        let pub_in_self_ok = self_restricted::value() == 6;
        fn empty_turbofish_helper() -> i32 {
            7
        }
        let empty_turbofish_ok =
            empty_turbofish_helper::<>() == 7;
        fn empty_where_helper() -> i32 where {
            8
        }
        let empty_where_ok = empty_where_helper() == 8;
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
        struct CfgPattern {
            #[cfg(any())]
            AliasCollisionParser_PATTERN_CFG: i32,
        }
        let cfg_pattern = CfgPattern {};
        let match_pattern_cfg_ok = Some(match cfg_pattern {
            CfgPattern {
                #[cfg(any())]
                AliasCollisionParser_PATTERN_CFG,
            } if AliasCollisionParser_PATTERN_CFG == Self::PATTERN_CFG => true,
            _ => false,
        }) == Some(true);
        let CfgPattern {
            #[cfg(any())]
            AliasCollisionParser_PATTERN_CFG,
        } = CfgPattern {};
        let pattern_cfg_ok =
            AliasCollisionParser_PATTERN_CFG == Self::PATTERN_CFG;
        mod relative_alias {
            pub fn value() -> i32 {
                super::AliasCollisionParser_PARENT_MODULE
            }
        }
        let parent_module_alias_ok =
            relative_alias::value() == Self::PARENT_MODULE;
        fn apply<F: Fn(i32) -> i32>(
            AliasCollisionParser_PARAM: i32,
            function: F,
        ) -> i32 {
            function(AliasCollisionParser_PARAM)
        }
        fn cfg_parameter(
            #[cfg(any())] AliasCollisionParser_CFG_PARAMETER: i32,
        ) -> i32 {
            AliasCollisionParser_CFG_PARAMETER
        }
        let cfg_parameter_ok =
            cfg_parameter() == Self::CFG_PARAMETER;
        #[cfg(any())]
        const AliasCollisionParser_CFG_ITEM: i32 = 1;
        let cfg_item_ok =
            AliasCollisionParser_CFG_ITEM == Self::CFG_ITEM;
        fn cfg_const_generic<
            #[cfg(any())] const AliasCollisionParser_CFG_CONST_GENERIC: usize,
        >() -> i32 {
            AliasCollisionParser_CFG_CONST_GENERIC
        }
        let cfg_const_generic_ok =
            cfg_const_generic() == Self::CFG_CONST_GENERIC;
        let cfg_closure =
            |#[cfg(any())] AliasCollisionParser_CFG_CLOSURE: i32|
                AliasCollisionParser_CFG_CLOSURE;
        let cfg_closure_ok =
            cfg_closure() == Self::CFG_CLOSURE;
        before_scope == SCOPE
            && after_scope == SCOPE
            && if_binding_ok
            && for_binding_ok
            && match_binding_ok
            && turbofish_match_binding_ok
            && closure_match_binding_ok
            && active_cfg_use_ok
            && inactive_cfg_use_ok
            && active_cfg_let_ok
            && inactive_cfg_let_ok
            && duplicate_cfg_ok
            && staged_cfg_before
            && staged_cfg_after
            && leading_match_binding_ok
            && block_match_binding_ok
            && not_equal_alias_ok
            && if_head_alias_ok
            && while_head_alias_ok
            && if_let_constant_ok
            && while_let_constant_ok
            && match_head_alias_ok
            && let_chain_binding_ok
            && nonleading_let_chain_ok
            && const_block_binding_ok
            && cfg_attr_alias_ok
            && action_cfg_alias_ok
            && type_only_import_alias_ok
            && value_import_alias_ok
            && inline_const_alias_ok
            && associated_const_alias_ok
            && format_capture_ok
            && standard_format_capture_ok
            && cfg_disabled_format_ok
            && format_local_ok
            && c_strings_ok
            && underscore_assignment_ok
            && opaque_receiver_tokens_ok
            && opaque_attribute_receiver_tokens_ok
            && unsafe_extern_alias_ok
            && safe_foreign_alias_ok
            && raw_lifetime_ok
            && placeholder_lifetime_ok
            && raw_reference_ok
            && nested_raw_reference_ok
            && exponent_underscore_ok
            && one_sided_range_patterns_ok
            && attributed_binder_ok
            && const_generic_default_ok
            && safe_identifier_ok
            && unicode_escape_underscores_ok
            && multiple_match_inner_attrs_ok
            && non_bmp_identifier_ok
            && raw_macro_ok
            && raw_macro_name_ok
            && raw_string_macro_ok
            && unicode_macro_name_ok
            && opaque_macro_alias_ok
            && commented_macro_header_ok
            && module_item_macro_ok
            && impl_item_macro_ok
            && shadowed_macro_ok
            && qualified_macro_ok
            && imported_macro_ok
            && standard_qualified_macro_ok
            && custom_matches_macro_ok
            && opaque_type_macro_ok
            && opaque_pattern_macro_ok
            && macro_ident_ok
            && unicode_identifiers_ok
            && input_facade_ok
            && leading_or_alias_ok
            && matches_binding_ok
            && nested_closure_binding_ok
            && const_generic_ok
            && associated_type_bound_ok
            && tuple_field_ok
            && pub_self_ok
            && pub_in_self_ok
            && empty_turbofish_ok
            && empty_where_ok
            && impl_const_generic_ok
            && braced_parameter_ok
            && match_pattern_cfg_ok
            && pattern_cfg_ok
            && parent_module_alias_ok
            && cfg_parameter_ok
            && cfg_item_ok
            && cfg_const_generic_ok
            && cfg_closure_ok
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
        | FORMAT_CAPTURE
        | FORMAT_LOCAL
        | QUALIFIED_MACRO
        | CONST_CHAIN
        | TYPE_ONLY
        | VALUE_IMPORT
        | UNSAFE_EXTERN
        | TURBOFISH
        | STANDARD_FORMAT
        | SAFE_FOREIGN
        | RAW_LIFETIME
        | OPAQUE_MACRO
        | CUSTOM_MATCHES
        | CLOSURE_MATCH
        | ACTIVE_CFG_USE
        | INACTIVE_CFG_USE
        | ACTIVE_CFG_LET
        | INACTIVE_CFG_LET
        | DUPLICATE_CFG
        | IMPORTED_MACRO
        | TYPE_MACRO
        | PATTERN_MACRO
        | CFG_FORMAT
        | RAW_MACRO_NAME
        | STAGED_CFG
        | CFG_PARAMETER
        | RAW_STRING_MACRO
        | PATTERN_CFG
        | ASSOCIATED_BOUND
        | PARENT_MODULE
        | CFG_ITEM
        | CFG_CONST_GENERIC
        | CFG_CLOSURE
        | UNICODE_MACRO_NAME
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
FORMAT_CAPTURE: 'format_capture';
FORMAT_LOCAL: 'format_local';
QUALIFIED_MACRO: 'qualified_macro';
CONST_CHAIN: 'const_chain';
TYPE_ONLY: 'type_only';
VALUE_IMPORT: 'value_import';
UNSAFE_EXTERN: 'unsafe_extern';
TURBOFISH: 'turbofish';
STANDARD_FORMAT: 'standard_format';
SAFE_FOREIGN: 'safe_foreign';
RAW_LIFETIME: 'raw_lifetime';
OPAQUE_MACRO: 'opaque_macro';
CUSTOM_MATCHES: 'custom_matches';
CLOSURE_MATCH: 'closure_match';
ACTIVE_CFG_USE: 'active_cfg_use';
INACTIVE_CFG_USE: 'inactive_cfg_use';
ACTIVE_CFG_LET: 'active_cfg_let';
INACTIVE_CFG_LET: 'inactive_cfg_let';
DUPLICATE_CFG: 'duplicate_cfg';
IMPORTED_MACRO: 'imported_macro';
TYPE_MACRO: 'type_macro';
PATTERN_MACRO: 'pattern_macro';
CFG_FORMAT: 'cfg_format';
RAW_MACRO_NAME: 'raw_macro_name';
STAGED_CFG: 'staged_cfg';
CFG_PARAMETER: 'cfg_parameter';
RAW_STRING_MACRO: 'raw_string_macro';
PATTERN_CFG: 'pattern_cfg';
ASSOCIATED_BOUND: 'associated_bound';
PARENT_MODULE: 'parent_module';
CFG_ITEM: 'cfg_item';
CFG_CONST_GENERIC: 'cfg_const_generic';
CFG_CLOSURE: 'cfg_closure';
UNICODE_MACRO_NAME: 'unicode_macro_name';
ID: [a-z]+;
WS: [ \t\r\n]+ -> skip;
