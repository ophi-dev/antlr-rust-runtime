grammar T;

expr
    : relation EOF
    ;

relation
    : calc
    | relation op = ('<' | '<=' | '>=' | '>' | '==' | '!=' | 'in') relation
    ;

calc
    : unary
    | calc op = ('*' | '/' | '%') calc
    | calc op = ('+' | '-') calc
    ;

// The optional labeled PLUS is followed by an unlabeled PLUS that would slide
// into `.nth(0)` whenever the labeled one is absent — the accessor must be
// omitted (`shadowed_when_absent` in context_label_selector).
shadowed
    : lead = PLUS? PLUS unary
    ;

unary
    : IDENT
    | NUM
    ;

// Issue #201, shape 1: the labels sit *inside* an unlabeled grouping block, so
// collapsing the block into one token-group ref would swallow them. Mirrors
// avdl's `(doc=DocComment)?` and `(oneway=Oneway | Throws errors+=...)?`.
grouped
    : (doc = IDENT)? (oneway = STAR | IN errors += unary (COMMA errors += unary)*)? NUM
    ;

// Issue #201, shape 2: `name` and `errors` label the same rule, mixing a single
// and a list label, so each accessor must resolve past the other's children.
// Mirrors avdl's `messageDeclaration`; `param` stands in for its
// `formalParameter`, keeping the `unary` count before `errors` exact.
mixed
    : name = unary LPAREN param* RPAREN (IN errors += unary (COMMA errors += unary)*)?
    ;

// A variable count of the label's own target in front of it leaves no fixed
// `.skip(N)`, so the list accessor must be declined even though the labels
// themselves are unambiguous.
mixed_unbounded
    : unary* IN errors += unary
    ;

param
    : IDENT
    ;

// Only one branch of the choice supplies `pick`, and its sibling branch matches
// the same rule unlabeled at the same flattened position — `.nth(0)` could read
// the sibling's child, so no accessor may be emitted.
branch_hazard
    : (pick = unary | STAR unary) NUM
    ;

// Same target labeled differently per branch: neither label may read the
// other branch's child.
branch_rival
    : (left = unary | right = unary) NUM
    ;

// An *exhaustive* choice contributes exactly one `unary` however it branches, so
// the following label is reliably the second one and keeps its accessor.
exhaustive_prefix
    : (first = unary | second = unary) pick = unary NUM
    ;

// A repeated scalar label over mutually exclusive branches: ANTLR overwrites a
// scalar on every iteration, so the merged read must select the *last* match. The
// trailing `LPAREN` is outside the accessor's token union, so nothing can become
// the `last()` ahead of the label — with a trailing `NUM` it could, and the merge
// would have to decline.
merged_repeats
    : (last_pick = IDENT+ | last_pick = NUM+) LPAREN
    ;

// One label declared over mutually exclusive branches at the same occurrence: the
// accessor's unioned token set selects whichever token the parse bound, so a
// single positional read serves both.
merged_rivals
    : (pick = IDENT | pick = NUM) NUM
    ;

// A label behind a sibling branch: restricting to the label's own path leaves one
// preceding `unary`, so the accessor is emitted at that fixed position.
path_restricted
    : (unary tail = unary | NUM) NUM
    ;

// Nested exhaustive choices: every path still yields exactly one `unary` before
// the label, so the inner choice's agreed count rolls up into the outer branch
// and the accessor survives.
nested_exhaustive_prefix
    : ((one = unary | two = unary) | three = unary) chosen = unary NUM
    ;

// The same choice made *optional*: it may now contribute no `unary` at all, so
// the following label has no fixed position and loses its accessor. Only the
// branch-local cardinality separates this from `exhaustive_prefix` above.
optional_prefix
    : (early = unary | late = unary)? tail = unary NUM
    ;

// A preceding token group overlaps the label's token type, so only some parses
// put a matching child ahead of it — no fixed occurrence, so no accessor.
overlapping_group
    : (IDENT | NUM) tail = IDENT
    ;

// Extra grouping levels are syntactically inert, so a label buried under them
// must still defeat the token-group collapse that would discard it.
nested_group
    : ((deep = IDENT))? NUM
    ;

// The outer `?` is satisfied wherever the label binds, but the *inner* `+` is a
// closed repeated group: it ran an unknown number of times, so the count of
// matching children ahead of the label is not fixed and the accessor must go.
// Only the repeated-group carve-out separates this from `nested_group`'s
// all-shared-groups case.
closed_repeat_prefix
    : ((IDENT STAR)+ tail = IDENT)? NUM
    ;

// Restricting to the label's path drops the *outer* choice, so the surviving
// inner choice must be read with its own arity of three. Treating it as an
// exhaustive two-way choice would fix the prefix count at one and emit
// `.nth(1)`, which reads nothing on the `param` path.
inner_choice_arity
    : ((unary | unary | param) tail = unary | NUM) NUM
    ;

// Issue #214, shape 1: the labeled group and the following union member are
// inside the same optional block. If the label is absent because the block was
// skipped, the follower is absent too, so it cannot slide into the label's slot.
shared_optional_block
    : (shared = (PLUS | MINUS) STAR NUM)?
    | shared = (STAR | SLASH) NUM
    ;

// Sharing an enclosing optional block is not enough when the labeled element
// has its own `?`: the block may run, omit the label, and still emit `STAR`.
direct_optional_in_shared_block
    : (shared = (PLUS | MINUS)? STAR NUM)?
    | shared = (STAR | SLASH) NUM
    ;

// Issue #214, shape 2: the first alternative needs last-assignment-wins while
// the second has one occurrence. With no later union member, `last()` serves both.
mixed_repetition
    : latest = (PLUS | MINUS)+ NUM
    | latest = (STAR | SLASH) NUM
    ;

// The same merge after one union-matching prefix proves the shared occurrence
// is preserved rather than assuming every compatible selector starts at zero.
prefixed_mixed_repetition
    : PLUS latest = (PLUS | MINUS)+ NUM
    | STAR latest = (STAR | SLASH) NUM
    ;

// The non-repeated alternative cannot be promoted to `last()` when a later
// union member would replace the token it bound.
mixed_repetition_followed
    : latest = (PLUS | MINUS)+ NUM
    | latest = (STAR | SLASH) STAR NUM
    ;

LESS
    : '<'
    ;

LESS_EQUALS
    : '<='
    ;

GREATER_EQUALS
    : '>='
    ;

GREATER
    : '>'
    ;

EQUALS
    : '=='
    ;

NOT_EQUALS
    : '!='
    ;

IN
    : 'in'
    ;

STAR
    : '*'
    ;

SLASH
    : '/'
    ;

PERCENT
    : '%'
    ;

PLUS
    : '+'
    ;

MINUS
    : '-'
    ;

IDENT
    : [a-zA-Z_] [a-zA-Z0-9_]*
    ;

NUM
    : [0-9]+
    ;

// Declared last so the issue #201 rules above do not renumber the token types
// the pre-existing context snapshots pin.
COMMA
    : ','
    ;

LPAREN
    : '('
    ;

RPAREN
    : ')'
    ;

WS
    : [ \t\r\n]+ -> skip
    ;
