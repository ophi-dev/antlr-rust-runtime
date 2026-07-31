// A user-authored module item wins over a metadata-shaped compatibility alias.
grammar AliasCollision;

@parser::members {
    struct AliasCollisionParser_ID;
    use antlr4_runtime::TOKEN_EOF as AliasCollisionParser_EOF;
}

start
    : {
        let _user_symbol = AliasCollisionParser_ID;
        let _user_import = AliasCollisionParser_EOF;
        true
    }? ID EOF
    ;

ID: [a-z]+;
WS: [ \t\r\n]+ -> skip;
