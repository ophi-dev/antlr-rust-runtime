// Pinned from Perses commit 29f9553367654ca682a8aca40b92ce5139114a7e.
// Locally modified to remove Java semantics and recognize modern Rust syntax.
// See README.md for provenance, licensing, and the exact adaptation.
grammar Rust;

fragment XID_Start :
    [_\p{XID_Start}];

fragment XID_Continue:
    [\p{XID_Continue}];

// === Modules and items

crate:
    mod_body EOF;

mod_body:
    inner_attr*  item*;

visibility:
    'pub' visibility_restriction?
    | 'crate'  //experimental, issue 46209
    ;

// Note that `pub(` does not necessarily signal the beginning of a visibility
// restriction! For example:
//
//     struct T(i32, i32, pub(i32));
//
// Here the `(` is part of the type `(i32)`.
visibility_restriction:
    '(' 'crate' ')'
    | '(' 'self' ')'
    | '(' 'super' ')'
    | '(' 'in' simple_path ')';

item:
    attr* visibility? pub_item
    | attr* impl_block
    | attr* extern_mod
    | attr* macro_iterm
    | attr* '\''; // experimental ignore-tidy-cr
//TODO: attr* need to be moved to somewhere else here


pub_item
    : extern_crate     // `pub extern crate` is deprecated but still allowed
    | use_decl
    | mod_decl_short
    | mod_decl
    | static_decl
    | const_decl
    | associated_const_decl //experimental
    | associated_static_decl //experimental
    | fn_decl
    | type_decl
    | struct_decl
    | enum_decl
    | union_decl
    | trait_decl
    | trait_alias
    | macro_decl
    ;



// --- extern crate

extern_crate:
    'extern' 'crate' (ident|'self') rename? ';'; //experimental: extern-crate-self-pass


// --- use declarations

use_decl:
    'use' use_path ';';

use_path:
    '::'? '{' use_item_list? '}'
    | '::'? (any_ident|'*') ('::' any_ident)* use_suffix?;

use_suffix:
    '::' '*'
    | '::' '{' use_item_list? '}'
    | rename;

use_item:
    (any_ident | use_path | '*') rename?;

use_item_list:
    use_item (',' use_item)* ','?;

rename:
    'as' (ident | '_');


// --- Modules

mod_decl_short:
    'mod' ident ';';

mod_decl:
    'mod' ident '{' mod_body '}';


// --- Foreign modules

extern_mod:
    'unsafe'? extern_abi '{' inner_attr* foreign_item* '}';

foreign_item:
    attr* visibility? foreign_item_tail
    | attr* macro_invocation_semi;

foreign_item_tail:
    ('safe' | 'unsafe')? 'static' 'mut'? ident ':' type ('=' expr)? ';' // experimental: added ('=' expr)? . Syntactically, a foreign static may have a body.
    | 'type' ident type_parameters? colon_bound? where_clause? (':' type)? ('=' type)?';'
    | foreign_fn_decl;


// --- static and const declarations

static_decl:
    'static' 'mut'? ident ':' ty_sum '=' expr ';';

associated_static_decl:
    'static' 'mut'? ident ':' ty_sum';';

const_decl:
    'default'? 'const' (ident|'_') ':' ty_sum '=' expr ';';

associated_const_decl:
    'const' ident (':' ty_sum)? ';'; //experimental:  const ident syntactic but not semantically

// --- Functions

fn_decl:
    fn_head '(' param_list? ')' fn_rtype? where_clause? ( block_with_inner_attrs | ';'); //experimental for supporting `fn` forms having or lacking a body are syntactically valid.

method_decl:
    fn_head '(' method_param_list? ')' fn_rtype? where_clause? ( block_with_inner_attrs | ';');//experimental for supporting `fn` forms having or lacking a body are syntactically valid.

trait_method_decl:
    fn_head '(' trait_method_param_list? ')' rtype? where_clause? (block_with_inner_attrs | ';');

foreign_fn_decl:
    'safe'? fn_head '(' variadic_param_list? ')' rtype? where_clause? ( block_with_inner_attrs | ';');  //experimental for supporting `fn` forms having or lacking a body are syntactically valid.

//macro declaration here is not documented,
macro_decl:
     macro_head ( '(' tt* ')' )? fn_rtype? where_clause? tt; // tt* should be replaced onced offical grammar is released

macro_head:
    'macro' ident type_parameter?;
// Parts of a `fn` definition up to the type parameters.
//
// `const` and `extern` are incompatible on a `fn`, but this grammar
// does not rule it out, holding that in a hypothetical Rust language
// specification, it would be clearer to specify this as a semantic
// rule, not a syntactic one. That is, not every rule that can be
// enforced gramatically should be.
fn_head:
    ('async' | 'const' | 'unsafe')*extern_abi? 'fn' ident type_parameters?; //experimental Ensures that all `fn` forms can have all the function qualifiers syntactically.

param:
    attr* '...'
    | attr* mut_or_const? ~(EOF)? pattern ':' (param_ty|'...')
    | attr*  '&'? lifetime? mut_or_const?  'self' (':' type)?; // experimental:`self` is syntactically accepted

param_ty:
    ty_sum
    | 'impl' bound;  // experimental: feature(universal_impl_trait)

param_list:
    param (',' param)* (',' attr* pattern mut_or_const ':' '...')? ','?;

variadic_param_list:
     param (','  param)* (',' attr* '...')? ','?; //experimental c_variadic

variadic_param_list_names_optional:
    trait_method_param (',' trait_method_param)* (',' attr* '...')? ','?;

self_param:
    'mut'? 'self' (':' ty_sum)?
    | '&' lifetime? 'mut'? 'self';

method_param_list:
    (param | self_param) (',' param)* ','?;

// Argument names are optional in traits. The ideal grammar here would be
// `(pat ':')? ty_sum`, but parsing this would be unreasonably complicated.
// Instead, the `pat` is restricted to a few short, simple cases.
trait_method_param:
    attr* '...'
    | attr* ( ('(' (restricted_pat ',')* restricted_pat')' ) |  restricted_pat) ':' attr* ty_sum
    | attr* ty_sum;

restricted_pat:
    'ref'? ('&' | '&&' | 'mut')? ('_' | ident);

trait_method_param_list:
    attr* (trait_method_param | self_param) (',' trait_method_param)* ','?;

// `ty_sum` is permitted in parameter types (although as a matter of semantics
// an actual sum is always rejected later, as having no statically known size),
// but only `ty` in return types. This means that in the where-clause
// `where T: Fn() -> X + Clone`, we're saying that T implements both
// `Fn() -> X` and `Clone`, not that its return type is `X + Clone`.
rtype
    : '->' type
    ;

// Experimental `feature(conservative_impl_trait)`.
fn_rtype:
    '->' (type | 'impl' bound);


// --- type, struct, and enum declarations

type_decl:
    'type' ident type_parameters? where_clause? '=' ty_sum ';'
    | 'type' ident type_parameters? colon_bound? where_clause? (':' type)? ('=' type)?';'; //experimental:test_data/rust_programs/rust_testsuite/ui/parser/item-free-type-bounds-syntactic-pass.rs

struct_decl:
    'struct' ident type_parameters? struct_tail;

struct_tail:
    where_clause? ';'
    | '(' tuple_struct_field_list? ')' where_clause? ';'
    | where_clause? '{' field_decl_list? '}';

tuple_struct_field:
    attr* visibility? ty_sum;

tuple_struct_field_list:
    tuple_struct_field (',' tuple_struct_field)* ','?;

field_decl:
    attr* visibility? ident ':' ty_sum;

field_decl_list:
    field_decl (',' field_decl)* ','?;

enum_decl:
    'enum' ident type_parameters? where_clause? '{' enum_variant_list? '}';

enum_variant:
    attr* visibility? enum_variant_main ('=' lit)?;

enum_variant_list:
    enum_variant (',' enum_variant)* ','?;

enum_variant_main:
    ident '(' enum_tuple_field_list? ')'
    | ident '{' enum_field_decl_list? '}'
    | ident '=' expr
    | ident;

// enum variants that are tuple-struct-like can't have `pub` on individual fields.
enum_tuple_field:
    attr* ty_sum;

enum_tuple_field_list:
    enum_tuple_field (',' enum_tuple_field)* ','?;

// enum variants that are struct-like can't have `pub` on individual fields.
enum_field_decl:
    attr*
    visibility?
    ident ':' ty_sum;

enum_field_decl_list:
    enum_field_decl (',' enum_field_decl)* ','?;

union_decl:
    'union' ident type_parameters? where_clause? '{' field_decl_list '}';


// --- Traits

// The `auto trait` syntax is an experimental feature, `optin_builtin_traits`,
// also known as OIBIT.
trait_decl:
    'unsafe'? 'auto'? 'trait' ident type_parameters? colon_bound? where_clause? '{'inner_attr* trait_item* '}';

trait_alias
    : 'trait' ident type_parameters? '='
        (ty_sum where_clause? | where_clause)  ';'
    ;

trait_item:
    attr* 'default'? visibility? 'type' ident type_parameters? colon_bound? where_clause? ty_default? ';'
    | attr* 'default'? 'const' ident ':' ty_sum const_default? ';'  // experimental associated constants
    | attr* 'default'? visibility? trait_method_decl
    | attr* 'default'? visibility? macro_invocation_semi // experimental:accept visibilities on items in traits syntactically but not semantically.
    | 'default'? visibility? (const_decl|associated_const_decl); //experimental

ty_default:
    '=' ty_sum;

const_default:
    '=' expr;


// --- impl blocks

impl_block:
    //experimental: ?const parse only
    'default'? 'unsafe'? 'impl' type_parameters? '?'? 'const'? impl_what where_clause? '{' impl_item* '}';

impl_what:
    '!' ty_sum 'for' ty_sum
    | ty_sum 'for' ty_sum
    | ty_sum 'for' '..'
    | ident type_arguments
    | ty_sum
    ;

impl_item:
    (attr|inner_attr)* visibility? impl_item_tail;

impl_item_tail:
    'default'? method_decl
    | 'default'? 'type' ident type_parameters? where_clause? '=' ty_sum ';'
    | (const_decl | associated_const_decl)
     // experimental test_data/rust_programs/rust_testsuite/ui/parser/impl-item-type-no-body-pass.rs
     // and test_data/rust_programs/rust_testsuite/ui/parser/self-param-syntactic-pass.rs
    | 'type' ident type_parameters? colon_bound? where_clause? (':' type)? ('=' tt*)?';'
    | macro_invocation_semi;


// === Attributes and token trees

attr:
    '#' '[' tt* ']';

inner_attr:
    '#' '!' '[' tt* ']';

tt:
    ~('(' | ')' | '{' | '}' | '[' | ']')
    | tt_delimited;

tt_delimited:
    tt_parens
    | tt_brackets
    | tt_block;

tt_parens:
    '(' tt* ')';

tt_brackets:
    '[' tt* ']';

tt_block:
    '{' tt* '}';

//nothing to do with macro now. Need to be refined in future
macro_tail:
    '!' tt_delimited;


// === Paths
// (forward references: ty_sum, ty_args)

// This is very slightly different from the syntax read by rustc:
// whitespace is permitted after `self` and `super` in paths.
//
// In rustc, `self::x` is an acceptable path, but `self :: x` is not,
// because `self` is a strict keyword except when followed immediately
// by the exact characters `::`. Same goes for `super`. Pretty weird.
//
// So instead, this grammar accepts that `self` is a keyword, and
// permits it specially at the very front of a path. Whitespace is
// ignored. `super` is OK anywhere except at the end.
//
// Separately and more tentatively: in rustc, qualified paths are
// permitted in peculiarly constrained contexts. In this grammar,
// qualified paths are just part of the syntax of paths (for now -
// this is not clearly an OK change).

path:
    path_segment_no_super
    | path_parent? '::' path_segment_no_super;

path_parent:
    'self'
    | '<' ty_sum as_trait? '>'
    | path_segment
    | '::' path_segment
    | path_parent '::' path_segment;

as_trait:
    'as' ty_sum;

path_segment:
    path_segment_no_super
    | 'super';

path_segment_no_super:
    simple_path_segment ('::' type_arguments)?;

simple_path:
    '::'? simple_path_segment ( '::' simple_path_segment)*;

simple_path_segment:
    ident
    | 'self'
    | 'super'
    | 'Self'
    | 'crate'
    | '$crate';


// === Type paths
// (forward references: rtype, ty_sum, ty_args)

for_lifetimes:
    'for' '<' lifetime_def_list? '>';

lifetime_def_list:
    lifetime_def (',' lifetime_def)* ','?;

lifetime_def:
    attr* lifetime (':' lifetime_bound)?;

lifetime_bound:
    lifetime
    | lifetime_bound '+' lifetime;

type_path_main:
    ty_path_tail
    | ty_path_parent? '::' ty_path_tail;

ty_path_tail:
    (ident | 'Self') '(' ty_sum_list? ')' rtype?
    | ty_path_segment_no_super;

ty_path_parent:
    'self'
    | '<' ty_sum as_trait? '>'
    | type_path_segment
    | '::' type_path_segment
    | ty_path_parent '::' type_path_segment;

type_path_segment:
    ty_path_segment_no_super
    | 'super';

ty_path_segment_no_super:
    '(' (ident | 'Self')? ')' type_arguments?
    | (ident | 'Self'| '&' 'raw') type_arguments?;


// === Type bounds

where_clause:
    'where' where_bound_list?;

where_bound_list:
    where_bound (',' where_bound)* ','?;

where_bound:
    lifetime ':' lifetime_bound
    | for_lifetimes? type empty_ok_colon_bound ?;

empty_ok_colon_bound:
    ':' bound?;

colon_bound:
    ':' bound;

bound:
    prim_bound
    | bound '+' prim_bound
    | bound '<' (lifetime_param ',')* type_parameter_list '>'; //experimental for associated_type_bounds

prim_bound:
    | '?'? for_lifetimes? ('dyn' | 'impl')? type_path_main
    | lifetime;


// === Types and type parameters

type
    : type_no_bounds
    | '&&' lifetime? 'mut'? type          // meaning `& & ty`
    | impl_trait_type
    | trait_object_type
    | '{' expr '}'
    ;

type_no_bounds
    : impl_trait_type_one_bound
    | trait_object_type_one_bound
    | '(' ty_sum ')'                    // grouping (parens are ignored)
    | tuple_type
    | never_type
    | raw_pointer_type
    | reference_type
    | array_or_slice_type
    | inferred_type
    | bare_function_type
    | macro_invocation
    ;

inferred_type
    : '_'
    ;

array_or_slice_type
    : '[' ty_sum (';' expr)? ']'
    ;

reference_type
    : '&' lifetime? 'mut'? type
    ;

raw_pointer_type
    : '*' mut_or_const type
    ;

never_type
    : '!'
    ;

tuple_type
    : '(' ')'
    | '(' ty_sum ',' ty_sum_list? ')'
    ;

impl_trait_type
    : 'impl' type_param_bounds
    ;

impl_trait_type_one_bound
    : 'impl' trait_bound
    ;

trait_object_type_one_bound
    : 'dyn'? trait_bound
    ;

type_param_bounds
    : type_param_bound ('+' type_param_bound)* '+'?
    ;

type_param_bound
    : lifetime
    | trait_bound
    | 'use' '<' ((lifetime | ident | 'Self') (',' (lifetime | ident | 'Self'))* ','?)? '>'
    ;

trait_object_type
    : 'dyn'? type_param_bounds?
    ;

trait_bound
    : '?'? for_lifetimes? type_path_main
    | '(' '?'? for_lifetimes? type_path_main ')'
    ;

bare_function_type
    : for_lifetimes? 'unsafe'? extern_abi? 'fn' '(' variadic_param_list_names_optional? ')' rtype?
    ;

mut_or_const:
    'mut'
    | 'const';

extern_abi:
    'extern' StringLit?;

type_arguments:
    '<' '>'
    | '<' lifetime (',' (lifetime | type_argument))* ','? '>'
    | '<' (lifetime ',')* type_argument (',' type_argument)* ','? '>';

type_argument:
    ident '=' ty_sum
    | ident ':' type_param_bounds
    | ty_sum
    | BareIntLit
    | 'true'
    | 'false'
    ;

// TODO(cnsun): get rid of this.
ty_sum:
   'dyn'? type ('+' bound)?;

ty_sum_list:
    ty_sum (',' ty_sum)* ','?;

type_parameters:
    '<' lifetime_param_list '>'
    | '<' (lifetime_param ',')* type_parameter_list? '>';

lifetime_param:
    attr* 'const'? lifetime (':' lifetime_bound)?;

lifetime_param_list:
    lifetime_param (',' lifetime_param)* ','?;

type_parameter:
    attr* 'const' ident ':' type const_default?
    | attr* ident colon_bound? ty_default?
    | ty_sum;

type_parameter_list:
    type_parameter (',' type_parameter)* ','?;


// === Patterns

pattern:
    '|'? pattern_no_top_alt ('|' pattern_no_top_alt)*;

pattern_no_top_alt:
    pattern_without_mut
    | 'mut' ident ('@' pattern)?;

pat_ident:
    ('_' | 'ref' ident);

// A `pattern_without_mut` is a pattern that does not start with `mut`.
// It is distinct from `pattern` to rule out ambiguity in parsing the
// pattern `&mut x`, which must parse like `&mut (x)`, not `&(mut x)`.
pattern_without_mut:
    '_' // wildcard pattern
    | '..=' pat_range_end
    | '..' //experimental
	| ident ('@' match_pattern)?
    | ident ('@' '(' match_pattern ')' )
    | pat_lit //litreal pattern
    | pat_range_end '...' pat_range_end // range pattern
    | pat_range_end '..' pat_range_end  // experimental `feature(exclusive_range_pattern)`
    | pat_range_end '..'
    | pat_range_end '..=' pat_range_end
    | path macro_tail
    | (pat_ident ',')* pat_ident ('@' pattern)?
    | 'ref'? 'mut'? ident ('@' pattern)? //IDpattern
    | path '(' pat_list_with_dots? ')'
    | path '{' pat_fields? '}'
    | path  // BUG: ambiguity with bare ident case (above)
    | '(' pat_list_with_dots? ')'
    | '[' pattern ( ',' pattern )* ','? ']' // slice pattern
    | '['']'
    | '&' pattern_without_mut
    | '&' 'mut' pattern
    | '&&' pattern_without_mut   // `&& pat` means the same as `& & pat`
    | '&&' 'mut' pattern
    | 'box' pattern
    | '$' pattern;

pat_range_end:
    path
    | pat_lit;

pat_lit:
    '-'? lit;

pat_list:
    pattern (',' pattern)* ','?;

pat_list_with_dots:
    pat_list_dots_tail
    | match_pattern (',' pattern)* (',' pat_list_dots_tail?)?;

pat_list_dots_tail:
    '..' (',' pat_list)?;

// rustc does not accept `[1, 2, tail..,]` as a pattern, because of the
// trailing comma, but I don't see how this is justifiable.  The rest of the
// language is *extremely* consistent in this regard, so I allow the trailing
// comma here.
//
// This grammar does not enforce the rule that a given slice pattern must have
// at most one `..`.


pat_fields_left:
    (ident | BareIntLit | FullIntLit);

pat_fields:
    '..'
    | pat_fields_left ':' pattern (',' pat_fields_left ':' pattern)* (',' '..' | ','?)
    | pat_field (',' pat_field)* (',' '..' | ','?);

pat_field:
    attr* 'box'? 'ref'? 'mut'? ident
    | attr* ident ':' pattern;


// === Expressions

expr:
   ('&' 'raw')? mut_or_const? assign_expr;

expr_no_struct:
    ('&' 'raw')? mut_or_const? assign_expr_no_struct;

expr_list:
    expr (',' expr)* ','?;


// --- Blocks

// OK, this is super tricky. There is an ambiguity in the grammar for blocks,
// `{ stmt* expr? }`, since there are strings that match both `{ stmt expr }`
// and `{ expr }`.
//
// The rule in Rust is that the `{ stmt expr }` parse is preferred: the body
// of the block `{ loop { break } - 1 }` is a `loop` statement followed by
// the expression `-1`, not a single subtraction-expression.
//
// Agreeably, the rule to resolve such ambiguities in ANTLR4, as in JS regexps,
// is the same. Earlier alternatives that match are preferred over later
// alternatives that match longer sequences of source tokens.
block:
    '{' stmt* expr? '}';

// Shared by blocky_expr and fn_body; in the latter case, any inner attributes
// apply to the whole fn.
block_with_inner_attrs:
    '{' inner_attr* stmt* expr? '}';

stmt
    : ';'
    | item  // Statement macros are included here.
    | attr* 'let' match_pattern (':' type)? '=' expr 'else' block ';'
    | attr* 'let' match_pattern (':' type)? ('=' expr)? ';'
    | attr* blocky_expr
    | expr ';'
    | macro_invocation_semi
    ;

// Inner attributes in `match`, `while`, `for`, `loop`, and `unsafe` blocks are
// experimental, `feature(stmt_expr_attributes)`.
blocky_expr
    : block_with_inner_attrs
    | if_cond_or_pat block ('else'  if_cond_or_pat block)* ('else' block)?
    | 'match' expr_no_struct '{' inner_attr* match_arms? '}'
    | loop_label? while_cond_or_pat block_with_inner_attrs
    | loop_label? 'for' pattern 'in' expr_no_struct block_with_inner_attrs
    | loop_label? 'loop' block_with_inner_attrs
    | loop_label? block_with_inner_attrs
    | 'unsafe' block_with_inner_attrs
    | 'try' block_with_inner_attrs
    | 'async' block_with_inner_attrs
    | 'const' block_with_inner_attrs
    ;

if_cond_or_pat:
    'if' expr_no_struct
    | 'if' let_chain
    | 'if' 'let' pattern '=' expr;

while_cond_or_pat:
    'while' expr_no_struct
    | 'while' let_chain
    | 'while' 'let' pattern '=' expr;

let_chain:
    let_chain_let ('&&' let_chain_operand)+
    | cmp_expr_no_struct ('&&' cmp_expr_no_struct)* '&&' let_chain_let
      ('&&' let_chain_operand)*
    ;

let_chain_operand:
    let_chain_let
    | cmp_expr_no_struct
    ;

let_chain_let:
    'let' pattern '=' cmp_expr_no_struct
    ;

loop_label:
    lifetime ':';

match_arms:
    match_arm_intro blocky_expr ','? match_arms?
    | match_arm_intro expr (',' match_arms?)?;

match_arm_intro:
    attr* match_pattern match_if_clause? '=>';

match_pattern:
    '|'? pattern ('|' pattern)*
    ;

match_if_clause:
    'if' expr;


// --- Primary expressions

// Attributes on expressions are experimental.
// Enable with `feature(stmt_expr_attributes)`.
expr_attrs:
    attr attr*;

// Inner attributes in array and struct expressions are experimental.
// Enable with `feature(stmt_expr_attributes)`.
expr_inner_attrs:
    inner_attr inner_attr*;

prim_expr:
    prim_expr_no_struct
    | path '{' expr_inner_attrs? fields? '}';

prim_expr_no_struct
    : lit
    | 'self'
    | path macro_tail?
    // The next 3 productions match exactly `'(' expr_list ')'`,
    // but (e) and (e,) are distinct expressions, so match them separately
    | '(' expr_inner_attrs? ')'
    | '(' expr_inner_attrs? expr ')'
    | '(' expr_inner_attrs? expr ',' expr_list? ')'
    | '[' expr_inner_attrs? expr_list? ']'
    | '[' expr_inner_attrs? expr ';' expr ']'
    | 'static'? 'move'? closure_params closure_tail
    | 'async' 'move'? (blocky_expr | closure_params closure_tail)
    | blocky_expr
    | 'break' lifetime_or_expr? lit? item? expr? //experimental: label/loop break value
    | 'continue' lifetime?
    | 'return' expr? // this is IMO a rustc bug, should be expr_no_struct
    | 'yield' expr?
    ;

lit:
    'true'
    | 'false'
    | BareIntLit '.'
    | BareIntLit
    | FullIntLit
    | ByteLit
    | ByteStringLit
    | FloatLit
    | CharLit
    | StringLit
    | CStringLit;

closure_params
    : '|' '|'
    | '||'
    | '|_|'
    | '|' closure_param_list? '|';

closure_param:
    attr* pattern_no_top_alt (':' type)?;

closure_param_list:
    closure_param (',' closure_param)* ','?;

closure_tail
    : rtype? block
    | expr;

lifetime_or_expr:
    lifetime
    | expr_no_struct;

fields:
    struct_update_base
    | field (',' field)* (',' struct_update_base | ','?);

struct_update_base:
    '..' expr;  // this is IMO a bug in the grammar. should be or_expr or something.

field:
    expr_attrs* ident  // struct field shorthand (field and local variable have the same name)
    | expr_attrs* field_name ':' expr;

field_name:
    ident
    | BareIntLit;  // Allowed for creating tuple struct values.


// --- Operators

post_expr:
    prim_expr
    | post_expr post_expr_tail;

post_expr_tail:
    '?'
    | '[' expr ']'
    | '.' ident (('::' type_arguments)? '(' expr_list? ')')?
    | '.' BareIntLit
    | TupleIndex
    | '(' expr_list? ')';

pre_expr:
    post_expr
    | expr_attrs pre_expr
    | '-' pre_expr
    | '!' pre_expr
    | '&' 'raw' mut_or_const pre_expr
    | '&' 'mut'? pre_expr
    | '&&' 'mut'? pre_expr   // meaning `& & expr`
    | '*' pre_expr
    | 'box' pre_expr
    | 'in' expr_no_struct block;  // placement new - possibly not the final syntax

cast_expr:
    pre_expr
    | cast_expr 'as' ty_sum
    | cast_expr ':' ty_sum;  // experimental type ascription

mul_expr:
    cast_expr
    | mul_expr '*' cast_expr
    | mul_expr '/' cast_expr
    | mul_expr '%' cast_expr;

add_expr:
    mul_expr
    | add_expr '+' mul_expr
    | add_expr '-' mul_expr;

shift_expr:
    add_expr
    | shift_expr '<' '<' add_expr
    | shift_expr '>' '>' add_expr;

bit_and_expr:
    shift_expr
    | bit_and_expr '&' shift_expr;

bit_xor_expr:
    bit_and_expr
    | bit_xor_expr '^' bit_and_expr;

bit_or_expr:
    bit_xor_expr
    | bit_or_expr '|' bit_xor_expr;

cmp_expr:
    bit_or_expr
    | bit_or_expr ('==' | '!=' | '<' | '<='  | '>' | '>=') bit_or_expr;

and_expr:
    cmp_expr
    | and_expr '&&' cmp_expr;

or_expr:
    and_expr
    | or_expr '||' and_expr;

range_expr:
    or_expr
    | or_expr '..' or_expr?
    | or_expr '..=' or_expr?
    | '..' or_expr?
    | '..=' or_expr?;

assign_expr:
    range_expr
    | '_' '=' assign_expr
    | range_expr ('=' | '*=' | '/=' | '%=' | '+=' | '-='
                      | '<<=' | '>>=' | '&=' | '^=' | '|=' ) assign_expr;


// --- Copy of the operator expression syntax but without structs

post_expr_no_struct:
    prim_expr_no_struct
    | post_expr_no_struct post_expr_tail;

pre_expr_no_struct:
    post_expr_no_struct
    | expr_attrs pre_expr_no_struct
    | '-' pre_expr_no_struct
    | '!' pre_expr_no_struct
    | '&' 'raw' mut_or_const pre_expr_no_struct
    | '&' 'mut'? pre_expr_no_struct
    | '&&' 'mut'? pre_expr_no_struct   // meaning `& & expr`
    | '*' pre_expr_no_struct
    | 'box' pre_expr_no_struct;

cast_expr_no_struct:
    pre_expr_no_struct
    | cast_expr_no_struct 'as' ty_sum
    | cast_expr_no_struct ':' ty_sum;  // experimental type ascription

mul_expr_no_struct:
    cast_expr_no_struct
    | mul_expr_no_struct '*' cast_expr_no_struct
    | mul_expr_no_struct '/' cast_expr_no_struct
    | mul_expr_no_struct '%' cast_expr_no_struct;

add_expr_no_struct:
    mul_expr_no_struct
    | add_expr_no_struct '+' mul_expr_no_struct
    | add_expr_no_struct '-' mul_expr_no_struct;

shift_expr_no_struct:
    add_expr_no_struct
    | shift_expr_no_struct '<' '<' add_expr_no_struct
    | shift_expr_no_struct '>' '>' add_expr_no_struct;

bit_and_expr_no_struct:
    shift_expr_no_struct
    | bit_and_expr_no_struct '&' shift_expr_no_struct;

bit_xor_expr_no_struct:
    bit_and_expr_no_struct
    | bit_xor_expr_no_struct '^' bit_and_expr_no_struct;

bit_or_expr_no_struct:
    bit_xor_expr_no_struct
    | bit_or_expr_no_struct '|' bit_xor_expr_no_struct;

cmp_expr_no_struct:
    bit_or_expr_no_struct
    | ('&' 'raw')? mut_or_const? bit_or_expr_no_struct ('==' | '!=' | '<' | '<=' | '>' | '>' '=') ('&' 'raw')? mut_or_const? bit_or_expr_no_struct;

and_expr_no_struct:
    cmp_expr_no_struct
    | and_expr_no_struct '&&' cmp_expr_no_struct;

or_expr_no_struct:
    and_expr_no_struct
    | or_expr_no_struct '||' and_expr_no_struct;

range_expr_no_struct:
    or_expr_no_struct
    | or_expr_no_struct '..' or_expr_no_struct?
    | or_expr_no_struct '..=' or_expr_no_struct?
    | '..' or_expr_no_struct?
    | '..=' or_expr_no_struct?;

assign_expr_no_struct:
    range_expr_no_struct
    | '_' '=' assign_expr_no_struct
    | range_expr_no_struct ('=' | '*=' | '/=' | '%=' | '+=' | '-='
                                | '<<=' | '>' '>' '=' | '&=' | '^=' | '|=' |'>='|'<=') assign_expr_no_struct;


// === Tokens

// `auto`, `default`, and 'union' are identifiers, but in certain places
// they're specially recognized as keywords.
ident:
    Ident
    | 'auto'
    | 'default'
    | 'union'
    | 'try'
    | 'crate'
    | 'macro_rules'
    | 'raw'
    | 'safe'
    | RawIdentifier
    ;

any_ident:
    ident
    | 'crate'
    | 'Self'
    | 'self'
    | 'static'
    | 'super';


// TODO: tokens '<<' '>>' confilcts ty_args, type need to be refactored

//TOD0:classify this token

tokens_no_delimiters_cash:
    ~('(' | ')' | '{' | '}' | '[' | ']' | CashMoney);



tokens_no_delimiters_repetition_operators:
    ~('(' | ')' | '{' | '}' | '[' | ']' | '+'|'*'|'?');
//not complete all tokens need to be refactored

//token:
//    keyword_token
//    | ident
//    | lit
//    | lifetime
//    | punctuation
//    | delimiter;


//macro rules
macro_iterm:
    macro_rules_definition
    |macro_invocation_semi;


macro_rules_definition:
    'macro_rules' '!' (~EOF)? macro_rules_def; //experimental: relex syntax for allowing macro_rules declaration with invalid name or without name

macro_rules_def:
    '(' macro_rules?')' ';'
    | '[' macro_rules? ']' ';'
    | '{' macro_rules? '}' ;
 //experimental: relex syntax for allowing empty macro rules body

macro_rules:
    macro_rule ( ';' macro_rule)* ';'? ;

macro_rule:
    macro_matcher '=>' macro_transcriber;

macro_matcher:
    '(' macro_match* ')'
    |'[' macro_match* ']'
    |'{' macro_match* '}';

macro_match:
    tokens_no_delimiters_cash
    |macro_matcher
    |'$' ~(EOF) ':' macro_frag_spec
    |'$' '(' macro_match+ ')'  macro_rep_sep? macro_rep_op;


macro_frag_spec:
    ~EOF;
//TODO:
// should be
//    'block' | 'expr' | 'ident' | 'item' | 'lifetime' | 'literal'
//    | 'meta' | 'pat' | 'path' | 'stmt' | 'tt' | 'ty' | 'vis';
// but it will mismatch some IDs


macro_rep_sep:
    tokens_no_delimiters_repetition_operators;

macro_rep_op:
    '*' | '+' | '?';

macro_transcriber:
    delim_token_tree;

delim_token_tree:
    '(' tt* ')'
    | '[' tt* ']'
    | '{' tt* '}';


macro_invocation_semi:
    simple_path '!' '(' tt* ')' ';'
    | simple_path '!' '[' tt* ']' ';'
    | simple_path '!' '{' tt* '}' ;

macro_invocation:
    simple_path '!' delim_token_tree;

// `$` is recognized as a token, so it may be present in token trees,
// and `macro_rules!` makes use of it. But it is not mentioned anywhere
// else in this grammar.
CashMoney:
    '$';

RawIdentifier:
    'r#' IDENT
    ;

fragment IDENT:
    XID_Start XID_Continue*;

lifetime
    : Lifetime
    | '\'static'
    | '\'_'
    ;

Lifetime:
    [']('r#')? IDENT;

Ident:
    IDENT;

fragment SIMPLE_ESCAPE:
    '\\' [0nrt'"\\];

fragment CHAR:
    ~['"\r\n\\\ud800-\udfff]          // a single BMP character other than a backslash, newline, or quote
    | [\ud800-\udbff][\udc00-\udfff]  // a single non-BMP character (hack for Java)
    | SIMPLE_ESCAPE
    | '\\x' [0-7] [0-9a-fA-F]
    | '\\u{' [0-9a-fA-F] [0-9a-fA-F_]* '}';

CharLit:
    '\'' (CHAR | '"') '\'';

fragment OTHER_STRING_ELEMENT:
    '\''
    | '\\' '\r'? '\n' [ \t]*
    | '\r'
    | '\n';

fragment STRING_ELEMENT:
    CHAR
    | OTHER_STRING_ELEMENT;

fragment RAW_CHAR:
    ~[\ud800-\udfff]          // any BMP character
    | [\ud800-\udbff][\udc00-\udfff];  // any non-BMP character (hack for Java)

// Here we use a non-greedy match to implement the
// (non-regular) rules about raw string syntax.
fragment RAW_STRING_BODY:
    '"' RAW_CHAR*? '"'
    | '#' RAW_STRING_BODY '#';

StringLit:
    '"' STRING_ELEMENT* '"'
    | 'r' RAW_STRING_BODY;

fragment C_STRING_CHAR:
    ~["\\\r\u0000\ud800-\udfff]
    | [\ud800-\udbff][\udc00-\udfff];

fragment C_BYTE_ESCAPE:
    '\\' [nrt'"\\]
    | '\\x' ('0' [1-9a-fA-F] | [1-9a-fA-F] [0-9a-fA-F]);

fragment C_UNICODE_ESCAPE:
    '\\u{' (
        [1-9a-fA-F] C_UNICODE_HEX_TAIL_5
        | '0' '_'* [1-9a-fA-F] C_UNICODE_HEX_TAIL_4
        | '0' '_'* '0' '_'* [1-9a-fA-F] C_UNICODE_HEX_TAIL_3
        | '0' '_'* '0' '_'* '0' '_'* [1-9a-fA-F] C_UNICODE_HEX_TAIL_2
        | '0' '_'* '0' '_'* '0' '_'* '0' '_'* [1-9a-fA-F] C_UNICODE_HEX_TAIL_1
        | '0' '_'* '0' '_'* '0' '_'* '0' '_'* '0' '_'* [1-9a-fA-F] C_UNICODE_HEX_TAIL_0
    ) '}';

fragment C_UNICODE_HEX_TAIL_5:
    '_'* ([0-9a-fA-F] C_UNICODE_HEX_TAIL_4)?;

fragment C_UNICODE_HEX_TAIL_4:
    '_'* ([0-9a-fA-F] C_UNICODE_HEX_TAIL_3)?;

fragment C_UNICODE_HEX_TAIL_3:
    '_'* ([0-9a-fA-F] C_UNICODE_HEX_TAIL_2)?;

fragment C_UNICODE_HEX_TAIL_2:
    '_'* ([0-9a-fA-F] C_UNICODE_HEX_TAIL_1)?;

fragment C_UNICODE_HEX_TAIL_1:
    '_'* ([0-9a-fA-F] C_UNICODE_HEX_TAIL_0)?;

fragment C_UNICODE_HEX_TAIL_0:
    '_'*;

fragment C_STRING_ELEMENT:
    C_STRING_CHAR
    | C_BYTE_ESCAPE
    | C_UNICODE_ESCAPE
    | '\\' '\r'? '\n' [ \t]*;

fragment C_RAW_CHAR:
    ~[\r\u0000\ud800-\udfff]
    | [\ud800-\udbff][\udc00-\udfff];

fragment C_RAW_STRING_BODY:
    '"' C_RAW_CHAR*? '"'
    | '#' C_RAW_STRING_BODY '#';

CStringLit:
    'c"' C_STRING_ELEMENT* '"'
    | 'cr' C_RAW_STRING_BODY;

fragment BYTE:
    ' '               // any ASCII character from 32 (space) to 126 (`~`),
    | '!'             // except 34 (double quote), 39 (single quote), and 92 (backslash)
    | [#-&]
    | [(-[]
    | ']'
    | '^'
    | [_-~]
    | SIMPLE_ESCAPE
    | '\\x' [0-9a-fA-F][0-9a-fA-F];

ByteLit:
    'b\'' (BYTE | '"') '\'';

fragment BYTE_STRING_ELEMENT:
    BYTE
    | OTHER_STRING_ELEMENT;

fragment RAW_BYTE_STRING_BODY:
    '"' [\t\r\n -~]*? '"'
    | '#' RAW_BYTE_STRING_BODY '#';

ByteStringLit:
    'b"' BYTE_STRING_ELEMENT* '"'
    | 'br' RAW_BYTE_STRING_BODY;

fragment DEC_DIGITS:
    [0-9][0-9_]*;

// BareIntLit and FullIntLit both match '123'; BareIntLit wins by virtue of
// appearing first in the file. (This comment is to point out the dependency on
// a less-than-obvious ANTLR rule.)
BareIntLit:
    DEC_DIGITS;

fragment INT_SUFFIX:
    [ui] ('8'|'16'|'32'|'64'|'128'|'size');

FullIntLit:
    DEC_DIGITS INT_SUFFIX?
    | '0x' '_'* [0-9a-fA-F] [0-9a-fA-F_]* INT_SUFFIX?
    | '0o' '_'* [0-7] [0-7_]* INT_SUFFIX?
    | '0b' '_'* [01] [01_]* INT_SUFFIX?;

fragment EXPONENT:
    [Ee] [+-]? '_'* [0-9] [0-9_]*;

fragment FLOAT_SUFFIX:
    'f32'
    | 'f64';

// Some lookahead is required here. ANTLR does not support this
// except by injecting some Java code into the middle of the pattern.
//
// A floating-point literal may end with a dot, but:
//
// *   `100..f()` is parsed as `100 .. f()`, not `100. .f()`,
//     contrary to the usual rule that lexers are greedy.
//
// *   Similarly, but less important, a letter or underscore after `.`
//     causes the dot to be interpreted as a separate token by itself,
//     so that `1.abs()` parses a method call. The type checker will
//     later reject it, though.
//
FloatLit
    :
    ( DEC_DIGITS '.' [0-9] [0-9_]* EXPONENT? FLOAT_SUFFIX?
    | DEC_DIGITS EXPONENT FLOAT_SUFFIX?
    | DEC_DIGITS FLOAT_SUFFIX
    )
    ;

Whitespace:
    [ \t\r\n]+ -> skip;

LineComment:
    '//' ~[\r\n]* -> skip;

BlockComment:
    '/*' (~[*/] | '/'* BlockComment | '/'+ (~[*/]) | '*'+ ~[*/])* '*'+ '/' -> skip;

// Combining the leading dot with a tuple index prevents `pair.0.1` from
// becoming a single `FloatLit` token while preserving ordinary decimal floats.
TupleIndex:
    '.' DEC_DIGITS;

Shebang:
    '#!/' ~[\r\n]* -> skip;

// BUG: only ascii identifiers are permitted
// BUG: doc comments are ignored
// BUG: associated constants are not supported
// BUG: rename `lit` -> `literal`
// BUG: probably inner attributes are allowed in many more places
// BUG: refactor `use_path` syntax to be like `path`, remove `any_ident`
// BUG: `let [a, xs.., d] = out;` does not parse
// BUG: ambiguity between expression macros, stmt macros, item macros
