// Reduced from kaby76/AntlrExamples CSharp/CSharpLexer.g4 (the Kochurkin C#
// lexer lineage): interpolated-string tracking kept inline in
// `@lexer::members` with C# bodies, rather than externalized into a
// `superClass`. The bodies below are verbatim C#; issue #206's `stack_member`
// pattern lowering maps them to SemIR so this grammar needs no hooks.
lexer grammar CSharpInterpolation;

@lexer::members
{private int interpolatedStringLevel;
private Stack<bool> interpolatedVerbatiums = new Stack<bool>();
private Stack<int> curlyLevels = new Stack<int>();
private bool verbatium;
}

INTERPOLATED_REGULAR_STRING_START
    : '$"' { interpolatedStringLevel++; interpolatedVerbatiums.Push(false); verbatium = false; } -> pushMode(INTERPOLATION_STRING)
    ;

INTERPOLATED_VERBATIUM_STRING_START
    : '$@"' { interpolatedStringLevel++; interpolatedVerbatiums.Push(true); verbatium = true; } -> pushMode(INTERPOLATION_STRING)
    ;

IDENTIFIER
    : [a-zA-Z_] [a-zA-Z_0-9]*
    ;

WS
    : [ \t\r\n]+ -> skip
    ;

mode INTERPOLATION_STRING;

OPEN_BRACE_INSIDE
    : '{' { curlyLevels.Push(1); } -> skip, pushMode(DEFAULT_MODE)
    ;

VERBATIUM_DOUBLE_QUOTE_INSIDE
    : { verbatium }? '""'
    ;

DOUBLE_QUOTE_INSIDE
    : '"' { interpolatedStringLevel--; interpolatedVerbatiums.Pop(); verbatium = (interpolatedVerbatiums.Count > 0 ? interpolatedVerbatiums.Peek() : false); } -> popMode
    ;

REGULAR_STRING_INSIDE
    : { !verbatium }? ~('{' | '\\' | '"')+
    ;

VERBATIUM_INSIDE_STRING
    : { verbatium }? ~('{' | '"')+
    ;
