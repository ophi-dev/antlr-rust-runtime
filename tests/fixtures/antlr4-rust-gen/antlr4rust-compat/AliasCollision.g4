// A user-authored module item wins over a metadata-shaped compatibility alias.
grammar AliasCollision;

@parser::members {
    struct AliasCollisionParser_ID;
    use antlr4_runtime::TOKEN_EOF as AliasCollisionParser_EOF;
    use self::{AliasCollisionParser_MODULE as RenamedModule};
}

start
    : {
        let _user_symbol = AliasCollisionParser_ID;
        let _user_import = AliasCollisionParser_EOF;
        let _compat_alias = AliasCollisionParser_MODULE;
        let _renamed_import = RenamedModule;
        let AliasCollisionParser_LOCAL = 7;
        let _local_binding = AliasCollisionParser_LOCAL;
        true
    }? (ID | MODULE | LOCAL) EOF
    ;

MODULE: 'module';
LOCAL: 'local';
ID: [a-z]+;
WS: [ \t\r\n]+ -> skip;
