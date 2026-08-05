grammar CParser;

options {
    superClass = CParserBase;
}

translationUnit
    : {this.EnterScope();}
      {this.IsTypedefName()}? Identifier
      {this.ExitScope();}
      EOF
    ;

Identifier: [a-z]+;
WS: [ \t\r\n]+ -> skip;
