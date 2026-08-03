// Metadata-derived antlr4rust token aliases must not depend on another legacy
// receiver appearing in the same parser.
grammar AliasOnly;

start
    : {
        AliasOnlyParser_ID == ID && AliasOnlyParser_T__0 == T__0
    }? ID '!' EOF
    ;

ID: [a-z]+;
WS: [ \t\r\n]+ -> skip;
