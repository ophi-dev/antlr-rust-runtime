// A user-authored module item wins over a metadata-shaped compatibility alias.
grammar AliasCollision;

@parser::members {
    marker: i32 = AliasCollisionParser_FIELD_INIT;

    struct AliasCollisionParser_ID;
    struct AliasCollisionParser_NAMED {
        marker: i32,
    }
    struct __antlr4rust_token_aliases;
    use antlr4_runtime::TOKEN_EOF as AliasCollisionParser_EOF;
    use self::{AliasCollisionParser_MODULE as RenamedModule};
    use self::{AliasCollisionParser_MEMBER_ONLY as RenamedMemberOnly};
    use self::AliasCollisionParser_DIRECT;
    use ::{std::fmt};
    #[cfg(
        any()
    )]
    use antlr4_runtime::TOKEN_EOF as AliasCollisionParser_CFG;

    fn member_alias_matches(&self) -> bool {
        AliasCollisionParser_MODULE == Self::MODULE
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
        let named_struct = AliasCollisionParser_NAMED { marker: 14 };
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
            && named_struct.marker == 14
            && AliasCollisionParser_NAMED == NAMED
            && self.marker == Self::FIELD_INIT
            && explicit.AliasCollisionParser_FIELD == FIELD
            && shorthand.AliasCollisionParser_FIELD == FIELD
            && apply(11, |value| value) == 11
            && AliasCollisionParser_DIRECT == DIRECT
            && self.member_alias_matches()
            && MemberHelper::module_alias_matches()
    }? (
        ID
        | MODULE
        | MEMBER_ONLY
        | DIRECT
        | CFG
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
    ) EOF
    ;

crossBody
    : { AliasCollisionParser_CROSS == CROSS }? CROSS EOF
    ;

MODULE: 'module';
MEMBER_ONLY: 'member';
DIRECT: 'direct';
CFG: 'cfg';
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
ID: [a-z]+;
WS: [ \t\r\n]+ -> skip;
