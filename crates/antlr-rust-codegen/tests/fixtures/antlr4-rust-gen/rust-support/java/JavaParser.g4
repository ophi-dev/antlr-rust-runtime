parser grammar JavaParser;

options {
    tokenVocab = JavaLexer;
    superClass = JavaParserBase;
}

compilationUnit : { this.IsNotIdentifierAssign() }? IDENTIFIER EOF;
