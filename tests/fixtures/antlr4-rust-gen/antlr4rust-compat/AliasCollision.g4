// A user-authored module item wins over a metadata-shaped compatibility alias.
grammar AliasCollision;

@parser::members {
    struct AliasCollisionParser_ID;
    use antlr4_runtime::TOKEN_EOF as AliasCollisionParser_EOF;
    use self::{AliasCollisionParser_MODULE as RenamedModule};
    use self::{AliasCollisionParser_MEMBER_ONLY as RenamedMemberOnly};
    use ::{std::fmt};
    #[cfg(any())]
    use antlr4_runtime::TOKEN_EOF as AliasCollisionParser_CFG;
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
        before_scope == SCOPE && after_scope == SCOPE
    }? (ID | MODULE | MEMBER_ONLY | CFG | SCOPE | CROSS | LOCAL) EOF
    ;

crossBody
    : { AliasCollisionParser_CROSS == CROSS }? CROSS EOF
    ;

MODULE: 'module';
MEMBER_ONLY: 'member';
CFG: 'cfg';
SCOPE: 'scope';
CROSS: 'cross';
LOCAL: 'local';
ID: [a-z]+;
WS: [ \t\r\n]+ -> skip;
