// SPDX-License-Identifier: BSD-3-Clause
// Copyright (c) 2026 Konstantin Vyatkin
use super::*;

impl AliasBindingScopes {
    pub(crate) fn is_bound(&self, name: &str, position: usize) -> bool {
        self.scopes
            .get(name)
            .is_some_and(|scopes| scopes.iter().any(|scope| scope.contains(&position)))
    }

    fn record(&mut self, bindings: impl IntoIterator<Item = (String, usize)>, scope: Range<usize>) {
        for (name, position) in bindings {
            self.binding_positions.insert(position);
            self.scopes.entry(name).or_default().push(scope.clone());
        }
    }

    fn record_potential(
        &mut self,
        bindings: impl IntoIterator<Item = String>,
        scope: Range<usize>,
    ) {
        for name in bindings {
            self.scopes.entry(name).or_default().push(scope.clone());
        }
    }

    fn record_local_cfg_alias_fallback<'a>(
        &mut self,
        aliases: impl IntoIterator<Item = &'a String>,
        site: CfgAliasFallbackSite<'_>,
    ) {
        let CfgAliasFallbackSite {
            block,
            block_insertion,
            binding_insertion,
            active,
            kind,
        } = site;
        for alias in aliases {
            self.use_target_aliases.insert(alias.clone());
            let fallback = self
                .local_cfg_alias_fallbacks
                .entry((block.start, block.end, alias.clone()))
                .or_insert_with(|| LocalCfgAliasFallback {
                    block_insertion,
                    earliest_lexical: None,
                    item_insertion: None,
                    item_active_predicates: BTreeSet::new(),
                });
            fallback.block_insertion = fallback.block_insertion.min(block_insertion);
            match kind {
                CfgAliasBindingKind::Lexical => match &mut fallback.earliest_lexical {
                    Some(lexical) if binding_insertion == lexical.insertion => {
                        lexical.active_predicates.insert(active.to_owned());
                    }
                    Some(lexical) if binding_insertion > lexical.insertion => {}
                    _ => {
                        fallback.earliest_lexical = Some(LexicalCfgAliasFallback {
                            insertion: binding_insertion,
                            active_predicates: BTreeSet::from([active.to_owned()]),
                        });
                    }
                },
                CfgAliasBindingKind::Item => {
                    fallback.item_insertion = Some(
                        fallback
                            .item_insertion
                            .map_or(binding_insertion, |insertion| {
                                insertion.min(binding_insertion)
                            }),
                    );
                    fallback.item_active_predicates.insert(active.to_owned());
                }
            }
        }
    }

    fn finalize_local_cfg_alias_fallbacks(&mut self, token_alias_module: &str) {
        let mut local_grouped = BTreeMap::<(usize, String), BTreeSet<String>>::new();
        let mut import_grouped = BTreeMap::<(usize, String), BTreeSet<String>>::new();
        for ((_, _, alias), fallback) in std::mem::take(&mut self.local_cfg_alias_fallbacks) {
            if let Some(lexical) = fallback.earliest_lexical {
                let (insertion, active_predicates) = if fallback.item_active_predicates.is_empty() {
                    (lexical.insertion, lexical.active_predicates)
                } else {
                    (fallback.block_insertion, fallback.item_active_predicates)
                };
                local_grouped
                    .entry((insertion, combined_cfg_predicate(active_predicates)))
                    .or_default()
                    .insert(alias);
            } else {
                import_grouped
                    .entry((
                        fallback
                            .item_insertion
                            .expect("an item cfg fallback records its insertion"),
                        combined_cfg_predicate(fallback.item_active_predicates),
                    ))
                    .or_default()
                    .insert(alias);
            }
        }
        self.use_target_replacements
            .extend(
                local_grouped
                    .into_iter()
                    .map(|((insertion, active), aliases)| {
                        let mut text = String::new();
                        for alias in aliases {
                            let _ = writeln!(
                                text,
                                "#[cfg(not({active}))]\n\
                                 #[allow(non_snake_case, unused_variables)]\n\
                                 let {alias} = {token_alias_module}::{alias};"
                            );
                        }
                        RustReplacement {
                            range: insertion..insertion,
                            text,
                        }
                    }),
            );
        self.use_target_replacements
            .extend(
                import_grouped
                    .into_iter()
                    .map(|((insertion, active), aliases)| RustReplacement {
                        range: insertion..insertion,
                        text: format!(
                            "#[cfg(not({active}))]\n#[allow(unused_imports)]\n\
                     use {token_alias_module}::{{{}}};\n",
                            aliases.into_iter().collect::<Vec<_>>().join(", ")
                        ),
                    }),
            );
    }

    fn wrap_with_cfg_alias_fallbacks(
        &mut self,
        range: Range<usize>,
        fallbacks: BTreeMap<String, BTreeSet<String>>,
        token_alias_module: &str,
    ) {
        if fallbacks.is_empty() {
            return;
        }
        let mut opening = "{\n".to_owned();
        for (active, aliases) in fallbacks {
            for alias in aliases {
                self.use_target_aliases.insert(alias.clone());
                let _ = writeln!(
                    opening,
                    "#[cfg(not({active}))]\n\
                     #[allow(non_snake_case, unused_variables)]\n\
                     let {alias} = {token_alias_module}::{alias};"
                );
            }
        }
        self.use_target_replacements.push(RustReplacement {
            range: range.start..range.start,
            text: opening,
        });
        self.use_target_replacements.push(RustReplacement {
            range: range.end..range.end,
            text: "\n}".to_owned(),
        });
    }
}

pub(crate) fn combined_cfg_predicate(predicates: BTreeSet<String>) -> String {
    if predicates.len() == 1 {
        predicates
            .into_iter()
            .next()
            .expect("checked one cfg predicate")
    } else {
        format!(
            "any({})",
            predicates.into_iter().collect::<Vec<_>>().join(", ")
        )
    }
}

pub(crate) fn local_antlr4rust_alias_bindings(
    body: &str,
    lexemes: &[RustLexeme],
    token_aliases: &BTreeSet<String>,
    token_alias_module: &str,
    syntax: &rust_syntax::RustSyntax,
) -> io::Result<AliasBindingScopes> {
    let delimiters = RustDelimiterMap::new(lexemes);
    let mut bindings = AliasBindingScopes::default();
    let positions_by_offset = lexemes
        .iter()
        .enumerate()
        .map(|(position, lexeme)| (lexeme.start, position))
        .collect::<BTreeMap<_, _>>();
    for byte_start in syntax.value_binding_byte_starts() {
        let Some(&position) = positions_by_offset.get(&byte_start) else {
            continue;
        };
        if let Some(binding) = alias_binding(body, lexemes[position], position, token_aliases) {
            let block = delimiters.enclosing_block(position);
            if let Some(fallback) = syntax.value_binding_cfg_fallback(byte_start) {
                bindings.record_local_cfg_alias_fallback(
                    std::iter::once(&binding.0),
                    CfgAliasFallbackSite {
                        block: block.clone(),
                        block_insertion: block_cfg_fallback_insertion(lexemes, &block, &delimiters)
                            .unwrap_or(fallback.insertion),
                        binding_insertion: fallback.insertion,
                        active: &fallback.active_predicate,
                        kind: CfgAliasBindingKind::Item,
                    },
                );
            }
            bindings.record([binding], block);
        }
    }
    for binding in syntax.scoped_value_bindings() {
        let Some(&position) = positions_by_offset.get(&binding.declaration_start) else {
            continue;
        };
        if let Some(binding_name) = alias_binding(body, lexemes[position], position, token_aliases)
        {
            if let Some(fallback) = &binding.cfg_fallback
                && let Some(&insertion_position) = positions_by_offset.get(&fallback.insertion)
            {
                let block = delimiters.enclosing_block(insertion_position);
                bindings.record_local_cfg_alias_fallback(
                    std::iter::once(&binding_name.0),
                    CfgAliasFallbackSite {
                        block: block.clone(),
                        block_insertion: block_cfg_fallback_insertion(lexemes, &block, &delimiters)
                            .unwrap_or(fallback.insertion),
                        binding_insertion: fallback.insertion,
                        active: &fallback.active_predicate,
                        kind: CfgAliasBindingKind::Item,
                    },
                );
            }
            bindings.record(
                [binding_name],
                lexeme_range_for_bytes(lexemes, &binding.scope),
            );
        }
    }
    for (position, lexeme) in lexemes.iter().enumerate() {
        if lexeme.kind != RustLexemeKind::Identifier || !is_standalone_identifier(lexemes, position)
        {
            continue;
        }
        match lexeme_text(body, *lexeme) {
            "let" => {
                if let Some(start) = next_significant(lexemes, position)
                    && let Some(end) = find_top_level_lexeme(
                        body,
                        lexemes,
                        start,
                        TopLevelLexeme::Punctuation(b'='),
                    )
                    .or_else(|| {
                        find_top_level_lexeme(
                            body,
                            lexemes,
                            start,
                            TopLevelLexeme::Punctuation(b';'),
                        )
                    })
                {
                    let conditional_let = is_conditional_let(body, lexemes, position);
                    let mut pattern_bindings =
                        pattern_alias_bindings(body, lexemes, start, end, token_aliases, syntax);
                    if conditional_let {
                        pattern_bindings.retain(|(_, binding)| {
                            !is_bare_pattern_identifier(lexemes, start, end, *binding)
                        });
                    }
                    let outer_cfg = local_cfg_attributes(body, lexemes, position, &delimiters);
                    let fallback_insertion = outer_cfg
                        .as_ref()
                        .map_or(lexeme.start, |(attributes_start, _)| *attributes_start);
                    let mut cfg_fallbacks = BTreeMap::<String, Vec<&String>>::new();
                    for (name, binding) in &pattern_bindings {
                        let mut active_predicates = Vec::new();
                        if let Some((_, active)) = &outer_cfg {
                            active_predicates.push(active.clone());
                        }
                        if let Some(active) =
                            syntax.pattern_binding_cfg_predicate(lexemes[*binding].start)
                        {
                            active_predicates.push(active);
                        }
                        if let Some(active) = cfg_all_predicate(&active_predicates) {
                            cfg_fallbacks.entry(active).or_default().push(name);
                        }
                    }
                    if !cfg_fallbacks.is_empty() {
                        let block = delimiters.enclosing_block(position);
                        let block_insertion =
                            block_cfg_fallback_insertion(lexemes, &block, &delimiters)
                                .unwrap_or(fallback_insertion);
                        for (active, aliases) in cfg_fallbacks {
                            bindings.record_local_cfg_alias_fallback(
                                aliases,
                                CfgAliasFallbackSite {
                                    block: block.clone(),
                                    block_insertion,
                                    binding_insertion: fallback_insertion,
                                    active: &active,
                                    kind: CfgAliasBindingKind::Lexical,
                                },
                            );
                        }
                    }
                    let scope = if conditional_let {
                        conditional_let_scope(body, lexemes, end, &delimiters).unwrap_or(end..end)
                    } else {
                        find_top_level_lexeme(body, lexemes, end, TopLevelLexeme::Punctuation(b';'))
                            .map_or(end..end, |semicolon| {
                                semicolon + 1..delimiters.enclosing_block(position).end
                            })
                    };
                    bindings.record(pattern_bindings, scope);
                }
            }
            "for" => {
                if let Some(start) = next_significant(lexemes, position)
                    && let Some(end) = find_top_level_lexeme(
                        body,
                        lexemes,
                        start,
                        TopLevelLexeme::Identifier("in"),
                    )
                {
                    let pattern_bindings =
                        pattern_alias_bindings(body, lexemes, start, end, token_aliases, syntax);
                    let scope = delimiters
                        .control_flow_body_block(body, lexemes, end + 1)
                        .and_then(|open| delimiters.block_contents(open))
                        .unwrap_or(end..end);
                    if let Some(insertion) =
                        block_cfg_fallback_insertion(lexemes, &scope, &delimiters)
                    {
                        for (name, binding) in &pattern_bindings {
                            if let Some(active) =
                                syntax.pattern_binding_cfg_predicate(lexemes[*binding].start)
                            {
                                bindings.record_local_cfg_alias_fallback(
                                    std::iter::once(name),
                                    CfgAliasFallbackSite {
                                        block: scope.clone(),
                                        block_insertion: insertion,
                                        binding_insertion: insertion,
                                        active: &active,
                                        kind: CfgAliasBindingKind::Lexical,
                                    },
                                );
                            }
                        }
                    }
                    bindings.record(pattern_bindings, scope);
                }
            }
            "use" => {
                if let Some(start) = next_significant(lexemes, position)
                    && lexemes[start].kind != RustLexemeKind::Punctuation(b'<')
                    && let Some(end) = find_top_level_lexeme(
                        body,
                        lexemes,
                        start,
                        TopLevelLexeme::Punctuation(b';'),
                    )
                {
                    let source_start = lexemes[start].start;
                    let use_source = &body[source_start..lexemes[end].end];
                    let analysis = analyze_use_tree(use_source)?;
                    let scope = delimiters.enclosing_block(position);
                    let module_scope = scope == (0..lexemes.len());
                    let use_bindings = analysis
                        .binding_ranges
                        .iter()
                        .filter_map(|(name, range)| {
                            token_aliases.contains(name).then(|| {
                                positions_by_offset
                                    .get(&(source_start + range.start))
                                    .copied()
                                    .map(|position| (name.clone(), position))
                            })?
                        })
                        .collect::<Vec<_>>();
                    let shadowed_aliases = if analysis.contains_glob {
                        token_aliases.clone()
                    } else {
                        use_bindings.iter().map(|(name, _)| name.clone()).collect()
                    };
                    if let Some((attributes_start, active)) =
                        local_cfg_attributes(body, lexemes, position, &delimiters)
                    {
                        bindings.record_local_cfg_alias_fallback(
                            shadowed_aliases.iter(),
                            CfgAliasFallbackSite {
                                block: scope.clone(),
                                block_insertion: block_cfg_fallback_insertion(
                                    lexemes,
                                    &scope,
                                    &delimiters,
                                )
                                .unwrap_or(attributes_start),
                                binding_insertion: attributes_start,
                                active: &active,
                                kind: CfgAliasBindingKind::Item,
                            },
                        );
                    }
                    if analysis.contains_glob {
                        bindings.record_potential(shadowed_aliases, scope.clone());
                    }
                    bindings.record(use_bindings, scope);
                    for target in analysis.targets.into_iter().filter(|target| {
                        target.local_module_leaf && token_aliases.contains(&target.name)
                    }) {
                        if module_scope && target.binding.as_deref() == Some(target.name.as_str()) {
                            bindings.direct_alias_imports.insert(target.name.clone());
                        }
                        bindings.use_target_aliases.insert(target.name.clone());
                        bindings.use_target_replacements.push(RustReplacement {
                            range: source_start + target.range.start
                                ..source_start + target.range.end,
                            text: format!("{token_alias_module}::{}", target.name),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    collect_match_alias_bindings(
        body,
        lexemes,
        token_aliases,
        MatchAliasContext {
            token_alias_module,
            delimiters: &delimiters,
            syntax,
        },
        &mut bindings,
    );
    collect_matches_macro_alias_bindings(
        body,
        lexemes,
        token_aliases,
        MatchAliasContext {
            token_alias_module,
            delimiters: &delimiters,
            syntax,
        },
        &mut bindings,
    )?;
    collect_function_alias_bindings(
        body,
        lexemes,
        token_aliases,
        &delimiters,
        syntax,
        &mut bindings,
    );
    collect_closure_alias_bindings(
        body,
        lexemes,
        token_aliases,
        ClosureAliasContext {
            token_alias_module,
            delimiters: &delimiters,
            syntax,
        },
        &mut bindings,
    );
    bindings.finalize_local_cfg_alias_fallbacks(token_alias_module);
    Ok(bindings)
}

pub(crate) fn local_cfg_attributes(
    body: &str,
    lexemes: &[RustLexeme],
    item_position: usize,
    delimiters: &RustDelimiterMap,
) -> Option<(usize, String)> {
    let mut cursor = item_position;
    let mut attributes_start = None;
    while let Some(close) = previous_significant(lexemes, cursor) {
        if lexemes[close].kind != RustLexemeKind::Punctuation(b']') {
            break;
        }
        let Some(open) = delimiters.pairs[close] else {
            break;
        };
        let Some(hash) = previous_significant(lexemes, open) else {
            break;
        };
        if lexemes[open].kind != RustLexemeKind::Punctuation(b'[')
            || lexemes[hash].kind != RustLexemeKind::Punctuation(b'#')
            || next_significant(lexemes, hash) != Some(open)
        {
            break;
        }
        attributes_start = Some(hash);
        cursor = hash;
    }
    let start = attributes_start?;
    let predicates =
        member_cfg_predicates(&body[lexemes[start].start..lexemes[item_position].start]);
    Some((lexemes[start].start, cfg_all_predicate(&predicates)?))
}

pub(crate) fn is_conditional_let(body: &str, lexemes: &[RustLexeme], position: usize) -> bool {
    let Some(previous) = previous_significant(lexemes, position) else {
        return false;
    };
    if lexemes[previous].kind == RustLexemeKind::Identifier {
        return matches!(lexeme_text(body, lexemes[previous]), "if" | "while");
    }
    lexemes[previous].kind == RustLexemeKind::Punctuation(b'&')
        && previous_significant(lexemes, previous)
            .is_some_and(|before| lexemes[before].kind == RustLexemeKind::Punctuation(b'&'))
}

pub(crate) fn conditional_let_scope(
    body: &str,
    lexemes: &[RustLexeme],
    assignment: usize,
    delimiters: &RustDelimiterMap,
) -> Option<Range<usize>> {
    let body_open = delimiters.control_flow_body_block(body, lexemes, assignment + 1)?;
    let body_close = delimiters.pairs.get(body_open).copied().flatten()?;
    let scope_start = find_top_level_logical_and(body, lexemes, assignment + 1, body_open)
        .and_then(|and| next_significant(&lexemes[..body_open], and))
        .unwrap_or(body_open + 1);
    Some(scope_start..body_close)
}

pub(crate) fn find_top_level_logical_and(
    body: &str,
    lexemes: &[RustLexeme],
    start: usize,
    end: usize,
) -> Option<usize> {
    let mut search = start;
    while search < end {
        let first = find_top_level_lexeme(
            body,
            &lexemes[..end],
            search,
            TopLevelLexeme::Punctuation(b'&'),
        )?;
        if let Some(second) = next_significant(&lexemes[..end], first)
            && lexemes[second].kind == RustLexemeKind::Punctuation(b'&')
        {
            return Some(second);
        }
        search = first + 1;
    }
    None
}

#[derive(Debug)]
pub(crate) struct RustDelimiterMap {
    pub(crate) pairs: Vec<Option<usize>>,
    pub(crate) enclosing_blocks: Vec<Range<usize>>,
}

impl RustDelimiterMap {
    pub(crate) fn new(lexemes: &[RustLexeme]) -> Self {
        let mut pairs = vec![None; lexemes.len()];
        let mut stack = Vec::new();
        for (position, lexeme) in lexemes.iter().enumerate() {
            match lexeme.kind {
                RustLexemeKind::Punctuation(open @ (b'(' | b'[' | b'{')) => {
                    let close = match open {
                        b'(' => b')',
                        b'[' => b']',
                        b'{' => b'}',
                        _ => unreachable!("matched opening delimiter"),
                    };
                    stack.push((position, close));
                }
                RustLexemeKind::Punctuation(close @ (b')' | b']' | b'}')) => {
                    if let Some((open, expected)) = stack.pop()
                        && close == expected
                    {
                        pairs[open] = Some(position);
                        pairs[position] = Some(open);
                    }
                }
                _ => {}
            }
        }

        let root = 0..lexemes.len();
        let mut braces: Vec<usize> = Vec::new();
        let mut enclosing_blocks = vec![root.clone(); lexemes.len()];
        for (position, lexeme) in lexemes.iter().enumerate() {
            if lexeme.kind == RustLexemeKind::Punctuation(b'}') {
                braces.pop();
            }
            enclosing_blocks[position] = braces.last().map_or_else(
                || root.clone(),
                |&open| open + 1..pairs[open].unwrap_or(lexemes.len()),
            );
            if lexeme.kind == RustLexemeKind::Punctuation(b'{') && pairs[position].is_some() {
                braces.push(position);
            }
        }
        Self {
            pairs,
            enclosing_blocks,
        }
    }

    pub(crate) fn enclosing_block(&self, position: usize) -> Range<usize> {
        self.enclosing_blocks
            .get(position)
            .cloned()
            .unwrap_or(0..self.pairs.len())
    }

    fn control_flow_body_block(
        &self,
        body: &str,
        lexemes: &[RustLexeme],
        start: usize,
    ) -> Option<usize> {
        let expression_start = (start..lexemes.len())
            .find(|&position| lexemes[position].kind != RustLexemeKind::Trivia)?;
        let mut position = expression_start;
        while position < lexemes.len() {
            match lexemes[position].kind {
                RustLexemeKind::Punctuation(b'(' | b'[') => {
                    position = self.pairs[position]? + 1;
                    continue;
                }
                RustLexemeKind::Punctuation(b'{') => {
                    let expression_block = position == expression_start
                        || is_prefixed_expression_block(body, lexemes, position);
                    if expression_block {
                        position = self.pairs[position]? + 1;
                        continue;
                    }
                    return Some(position);
                }
                RustLexemeKind::Punctuation(b';' | b'}') => return None,
                _ => position += 1,
            }
        }
        None
    }

    fn block_contents(&self, open: usize) -> Option<Range<usize>> {
        (self.pairs.get(open).copied().flatten()? > open)
            .then(|| open + 1..self.pairs[open].expect("checked matching delimiter"))
    }

    fn expression_end(
        &self,
        lexemes: &[RustLexeme],
        start: usize,
        closure_parameters: &[Range<usize>],
    ) -> usize {
        let mut position = start;
        let mut generic_depth = 0_usize;
        while position < lexemes.len() {
            match lexemes[position].kind {
                RustLexemeKind::Punctuation(b'(' | b'[' | b'{') => {
                    if let Some(close) = self.pairs[position] {
                        position = close + 1;
                        continue;
                    }
                }
                RustLexemeKind::Punctuation(b'<')
                    if generic_depth > 0 || is_turbofish_open(lexemes, position) =>
                {
                    generic_depth += 1;
                }
                RustLexemeKind::Punctuation(b'>') if generic_depth > 0 => {
                    generic_depth -= 1;
                }
                RustLexemeKind::Punctuation(b',' | b';' | b')' | b']' | b'}')
                    if generic_depth == 0 =>
                {
                    if lexemes[position].kind == RustLexemeKind::Punctuation(b',')
                        && closure_parameters
                            .iter()
                            .any(|parameters| parameters.contains(&position))
                    {
                        position += 1;
                        continue;
                    }
                    return position;
                }
                _ => {}
            }
            position += 1;
        }
        position
    }
}

pub(crate) fn is_prefixed_expression_block(
    body: &str,
    lexemes: &[RustLexeme],
    open: usize,
) -> bool {
    let Some(previous) = previous_significant(lexemes, open) else {
        return false;
    };
    if lexemes[previous].kind == RustLexemeKind::Punctuation(b'!') {
        return true;
    }
    if lexemes[previous].kind != RustLexemeKind::Identifier {
        return false;
    }
    match lexeme_text(body, lexemes[previous]) {
        "const" | "async" | "unsafe" => true,
        "move" => previous_significant(lexemes, previous).is_some_and(|prefix| {
            lexemes[prefix].kind == RustLexemeKind::Identifier
                && lexeme_text(body, lexemes[prefix]) == "async"
        }),
        _ => false,
    }
}

#[derive(Clone, Copy)]
pub(crate) enum TopLevelLexeme {
    Identifier(&'static str),
    Punctuation(u8),
}

impl TopLevelLexeme {
    fn matches(self, body: &str, lexeme: RustLexeme) -> bool {
        match self {
            Self::Identifier(identifier) => {
                lexeme.kind == RustLexemeKind::Identifier && lexeme_text(body, lexeme) == identifier
            }
            Self::Punctuation(punctuation) => {
                lexeme.kind == RustLexemeKind::Punctuation(punctuation)
            }
        }
    }
}

pub(crate) fn find_top_level_lexeme(
    body: &str,
    lexemes: &[RustLexeme],
    start: usize,
    target: TopLevelLexeme,
) -> Option<usize> {
    let mut delimiters = Vec::new();
    for (position, lexeme) in lexemes.iter().enumerate().skip(start) {
        if delimiters.is_empty() && target.matches(body, *lexeme) {
            return Some(position);
        }
        match lexeme.kind {
            RustLexemeKind::Punctuation(b'(') => delimiters.push(b')'),
            RustLexemeKind::Punctuation(b'[') => delimiters.push(b']'),
            RustLexemeKind::Punctuation(b'{') => delimiters.push(b'}'),
            RustLexemeKind::Punctuation(close @ (b')' | b']' | b'}')) => {
                if delimiters.pop() != Some(close) {
                    return None;
                }
            }
            RustLexemeKind::Punctuation(b';') if delimiters.is_empty() => return None,
            _ => {}
        }
    }
    None
}

pub(crate) fn pattern_alias_bindings(
    body: &str,
    lexemes: &[RustLexeme],
    start: usize,
    end: usize,
    token_aliases: &BTreeSet<String>,
    syntax: &rust_syntax::RustSyntax,
) -> Vec<(String, usize)> {
    let type_annotation = find_pattern_type_annotation(body, &lexemes[..end], start);
    (start..type_annotation.unwrap_or(end))
        .filter_map(|position| {
            let lexeme = lexemes[position];
            (lexeme.kind == RustLexemeKind::Identifier
                && is_unqualified_identifier(lexemes, position)
                && !syntax.is_opaque_macro_identifier(lexeme.start)
                && (next_significant(lexemes, position) == type_annotation
                    || !is_pattern_path_or_field(lexemes, position)))
            .then(|| alias_binding(body, lexeme, position, token_aliases))
            .flatten()
        })
        .collect()
}

pub(crate) fn is_bare_pattern_identifier(
    lexemes: &[RustLexeme],
    start: usize,
    end: usize,
    identifier: usize,
) -> bool {
    (start..end)
        .filter(|position| lexemes[*position].kind != RustLexemeKind::Trivia)
        .eq([identifier])
}

pub(crate) fn find_pattern_type_annotation(
    body: &str,
    lexemes: &[RustLexeme],
    start: usize,
) -> Option<usize> {
    let mut search = start;
    loop {
        let colon =
            find_top_level_lexeme(body, lexemes, search, TopLevelLexeme::Punctuation(b':'))?;
        let previous_is_colon = previous_significant(lexemes, colon)
            .is_some_and(|previous| lexemes[previous].kind == RustLexemeKind::Punctuation(b':'));
        let next_is_colon = next_significant(lexemes, colon)
            .is_some_and(|next| lexemes[next].kind == RustLexemeKind::Punctuation(b':'));
        if !previous_is_colon && !next_is_colon {
            return Some(colon);
        }
        search = colon + 1;
    }
}

pub(crate) fn alias_binding(
    body: &str,
    lexeme: RustLexeme,
    position: usize,
    token_aliases: &BTreeSet<String>,
) -> Option<(String, usize)> {
    let identifier = rust_identifier_name(lexeme_text(body, lexeme));
    token_aliases
        .contains(identifier)
        .then(|| (identifier.to_owned(), position))
}

#[derive(Clone, Copy)]
pub(crate) struct MatchAliasContext<'a> {
    token_alias_module: &'a str,
    delimiters: &'a RustDelimiterMap,
    syntax: &'a rust_syntax::RustSyntax,
}

pub(crate) fn collect_match_alias_bindings(
    body: &str,
    lexemes: &[RustLexeme],
    token_aliases: &BTreeSet<String>,
    context: MatchAliasContext<'_>,
    bindings: &mut AliasBindingScopes,
) {
    let closure_parameters = context
        .syntax
        .closure_bindings()
        .iter()
        .filter_map(|closure| {
            let start = closure
                .parameter_ranges
                .iter()
                .map(|range| range.start)
                .min()?;
            let end = closure
                .parameter_ranges
                .iter()
                .map(|range| range.end)
                .max()?;
            Some(lexeme_range_for_bytes(lexemes, &(start..end)))
        })
        .collect::<Vec<_>>();
    for (position, lexeme) in lexemes.iter().enumerate() {
        if lexeme.kind != RustLexemeKind::Identifier
            || lexeme_text(body, *lexeme) != "match"
            || !is_standalone_identifier(lexemes, position)
        {
            continue;
        }
        let Some(open) = context
            .delimiters
            .control_flow_body_block(body, lexemes, position + 1)
        else {
            continue;
        };
        let Some(close) = context.delimiters.pairs[open] else {
            continue;
        };
        let mut expression_fallbacks = BTreeMap::<String, BTreeSet<String>>::new();
        let mut arm_start = open + 1;
        while arm_start < close {
            arm_start = (arm_start..close)
                .find(|&candidate| {
                    lexemes[candidate].kind != RustLexemeKind::Trivia
                        && lexemes[candidate].kind != RustLexemeKind::Punctuation(b',')
                })
                .unwrap_or(close);
            if arm_start >= close {
                break;
            }
            let Some(arrow) =
                find_match_arm_fat_arrow(lexemes, arm_start, close, context.delimiters)
            else {
                break;
            };
            let pattern_end = find_top_level_lexeme(
                body,
                &lexemes[..arrow],
                arm_start,
                TopLevelLexeme::Identifier("if"),
            )
            .unwrap_or(arrow);
            let pattern_bindings = match_pattern_alias_bindings(
                body,
                lexemes,
                arm_start,
                pattern_end,
                token_aliases,
                context.syntax,
            );
            for (name, binding) in &pattern_bindings {
                if let Some(active) = context
                    .syntax
                    .pattern_binding_cfg_predicate(lexemes[*binding].start)
                {
                    expression_fallbacks
                        .entry(active)
                        .or_default()
                        .insert(name.clone());
                }
            }
            let Some(body_start) = next_significant(lexemes, arrow) else {
                break;
            };
            let body_end = if lexemes[body_start].kind == RustLexemeKind::Punctuation(b'{') {
                context.delimiters.pairs[body_start].unwrap_or_else(|| {
                    context
                        .delimiters
                        .expression_end(lexemes, body_start, &closure_parameters)
                })
            } else {
                context
                    .delimiters
                    .expression_end(lexemes, body_start, &closure_parameters)
            }
            .min(close);
            bindings.record(pattern_bindings, arm_start..body_end);
            arm_start = body_end.saturating_add(1);
        }
        bindings.wrap_with_cfg_alias_fallbacks(
            lexemes[position].start..lexemes[close].end,
            expression_fallbacks,
            context.token_alias_module,
        );
    }
}

pub(crate) fn collect_matches_macro_alias_bindings(
    body: &str,
    lexemes: &[RustLexeme],
    token_aliases: &BTreeSet<String>,
    context: MatchAliasContext<'_>,
    bindings: &mut AliasBindingScopes,
) -> io::Result<()> {
    const PATTERN_PREFIX: &str = "match () { ";
    const PATTERN_SUFFIX: &str = " => (), _ => () }";

    for (position, lexeme) in lexemes.iter().enumerate() {
        if lexeme.kind != RustLexemeKind::Identifier || lexeme_text(body, *lexeme) != "matches" {
            continue;
        }
        let Some(bang) = next_significant(lexemes, position) else {
            continue;
        };
        let Some(open) = next_significant(lexemes, bang) else {
            continue;
        };
        if lexemes[bang].kind != RustLexemeKind::Punctuation(b'!')
            || !matches!(
                lexemes[open].kind,
                RustLexemeKind::Punctuation(b'(' | b'[' | b'{')
            )
        {
            continue;
        }
        if context.syntax.is_opaque_macro_byte(lexemes[open].start) {
            continue;
        }
        let Some(close) = context.delimiters.pairs[open] else {
            continue;
        };
        let Some(comma) = find_top_level_lexeme(
            body,
            &lexemes[..close],
            open + 1,
            TopLevelLexeme::Punctuation(b','),
        ) else {
            continue;
        };
        let Some(pattern_start) = next_significant(&lexemes[..close], comma) else {
            continue;
        };
        let trailing_comma = previous_significant(lexemes, close)
            .filter(|candidate| lexemes[*candidate].kind == RustLexemeKind::Punctuation(b','));
        let pattern_tail = trailing_comma.unwrap_or(close);
        let guard = find_top_level_lexeme(
            body,
            &lexemes[..pattern_tail],
            pattern_start,
            TopLevelLexeme::Identifier("if"),
        );
        let pattern_end = guard.unwrap_or(pattern_tail);
        if pattern_start >= pattern_end {
            continue;
        }

        let pattern_byte_start = lexemes[pattern_start].start;
        let pattern_byte_end = lexemes[pattern_end].start;
        let pattern_source = &body[pattern_byte_start..pattern_byte_end];
        let synthetic = format!("{PATTERN_PREFIX}{pattern_source}{PATTERN_SUFFIX}");
        let pattern_syntax = rust_syntax::analyze(&synthetic)?;
        let pattern_bindings = (pattern_start..pattern_end)
            .filter_map(|candidate| {
                let candidate_lexeme = lexemes[candidate];
                if candidate_lexeme.kind != RustLexemeKind::Identifier
                    || !is_unqualified_identifier(lexemes, candidate)
                {
                    return None;
                }
                let explicit_at_binding = next_significant(&lexemes[..pattern_end], candidate)
                    .is_some_and(|next| lexemes[next].kind == RustLexemeKind::Punctuation(b'@'));
                let explicit_modifier =
                    previous_significant(lexemes, candidate).is_some_and(|previous| {
                        previous >= pattern_start
                            && lexemes[previous].kind == RustLexemeKind::Identifier
                            && matches!(lexeme_text(body, lexemes[previous]), "ref" | "mut")
                    });
                let synthetic_start =
                    PATTERN_PREFIX.len() + candidate_lexeme.start - pattern_byte_start;
                let field_shorthand = pattern_syntax.is_pattern_field_shorthand(synthetic_start);
                (explicit_at_binding || explicit_modifier || field_shorthand)
                    .then(|| alias_binding(body, candidate_lexeme, candidate, token_aliases))
                    .flatten()
            })
            .collect::<Vec<_>>();
        let mut expression_fallbacks = BTreeMap::<String, BTreeSet<String>>::new();
        for (name, binding) in &pattern_bindings {
            let synthetic_start =
                PATTERN_PREFIX.len() + lexemes[*binding].start - pattern_byte_start;
            if let Some(active) = pattern_syntax.pattern_binding_cfg_predicate(synthetic_start) {
                expression_fallbacks
                    .entry(active)
                    .or_default()
                    .insert(name.clone());
            }
        }
        let guard_scope = guard
            .and_then(|guard| next_significant(&lexemes[..close], guard))
            .map_or(close..close, |start| start..close);
        bindings.record(pattern_bindings, guard_scope);
        let invocation_start = qualified_path_start(lexemes, position);
        bindings.wrap_with_cfg_alias_fallbacks(
            lexemes[invocation_start].start..lexemes[close].end,
            expression_fallbacks,
            context.token_alias_module,
        );
    }
    Ok(())
}

pub(crate) fn match_pattern_alias_bindings(
    body: &str,
    lexemes: &[RustLexeme],
    start: usize,
    end: usize,
    token_aliases: &BTreeSet<String>,
    syntax: &rust_syntax::RustSyntax,
) -> Vec<(String, usize)> {
    (start..end)
        .filter_map(|position| {
            let lexeme = lexemes[position];
            if lexeme.kind != RustLexemeKind::Identifier
                || !is_unqualified_identifier(lexemes, position)
            {
                return None;
            }
            let explicit_at_binding = next_significant(lexemes, position)
                .is_some_and(|next| lexemes[next].kind == RustLexemeKind::Punctuation(b'@'));
            let explicit_modifier =
                previous_significant(lexemes, position).is_some_and(|previous| {
                    lexemes[previous].kind == RustLexemeKind::Identifier
                        && matches!(lexeme_text(body, lexemes[previous]), "ref" | "mut")
                });
            let field_shorthand = syntax.is_pattern_field_shorthand(lexeme.start);
            (explicit_at_binding || explicit_modifier || field_shorthand)
                .then(|| alias_binding(body, lexeme, position, token_aliases))
                .flatten()
        })
        .collect()
}

pub(crate) fn find_match_arm_fat_arrow(
    lexemes: &[RustLexeme],
    start: usize,
    end: usize,
    delimiters: &RustDelimiterMap,
) -> Option<usize> {
    let mut position = start;
    while position < end {
        let lexeme = lexemes[position];
        if lexeme.kind == RustLexemeKind::Punctuation(b'=')
            && let Some(arrow) = next_significant(lexemes, position)
            && lexemes[arrow].kind == RustLexemeKind::Punctuation(b'>')
        {
            return Some(arrow);
        }
        if matches!(lexeme.kind, RustLexemeKind::Punctuation(b'(' | b'[' | b'{'))
            && let Some(close) = delimiters.pairs[position]
        {
            position = close + 1;
            continue;
        }
        position += 1;
    }
    None
}

#[derive(Clone, Copy)]
pub(crate) struct ClosureAliasContext<'a> {
    token_alias_module: &'a str,
    delimiters: &'a RustDelimiterMap,
    syntax: &'a rust_syntax::RustSyntax,
}

pub(crate) fn collect_closure_alias_bindings(
    body: &str,
    lexemes: &[RustLexeme],
    token_aliases: &BTreeSet<String>,
    context: ClosureAliasContext<'_>,
    bindings: &mut AliasBindingScopes,
) {
    for closure in context.syntax.closure_bindings() {
        let scope = lexeme_range_for_bytes(lexemes, &closure.scope);
        let block_fallback = function_body_cfg_fallback(lexemes, &scope, context.delimiters);
        let mut expression_fallbacks = BTreeMap::<String, BTreeSet<String>>::new();
        let mut closure_bindings = Vec::new();
        for parameter_bytes in &closure.parameter_ranges {
            let parameter = lexeme_range_for_bytes(lexemes, parameter_bytes);
            let parameter_bindings = pattern_alias_bindings(
                body,
                lexemes,
                parameter.start,
                parameter.end,
                token_aliases,
                context.syntax,
            );
            if let Some((_, active)) = closure
                .cfg_parameter_predicates
                .iter()
                .find(|(range, _)| range == parameter_bytes)
            {
                if let Some((block, insertion)) = &block_fallback {
                    bindings.record_local_cfg_alias_fallback(
                        parameter_bindings.iter().map(|(name, _)| name),
                        CfgAliasFallbackSite {
                            block: block.clone(),
                            block_insertion: *insertion,
                            binding_insertion: *insertion,
                            active,
                            kind: CfgAliasBindingKind::Lexical,
                        },
                    );
                } else {
                    expression_fallbacks
                        .entry(active.clone())
                        .or_default()
                        .extend(parameter_bindings.iter().map(|(name, _)| name.clone()));
                }
            }
            closure_bindings.extend(parameter_bindings);
        }
        if !expression_fallbacks.is_empty()
            && let (Some(first), Some(last)) = (
                first_significant_in(lexemes, scope.clone()),
                scope
                    .clone()
                    .rev()
                    .find(|position| lexemes[*position].kind != RustLexemeKind::Trivia),
            )
        {
            bindings.wrap_with_cfg_alias_fallbacks(
                lexemes[first].start..lexemes[last].end,
                expression_fallbacks,
                context.token_alias_module,
            );
        }
        bindings.record(closure_bindings, scope);
    }
}

pub(crate) fn collect_function_alias_bindings(
    body: &str,
    lexemes: &[RustLexeme],
    token_aliases: &BTreeSet<String>,
    delimiters: &RustDelimiterMap,
    syntax: &rust_syntax::RustSyntax,
    bindings: &mut AliasBindingScopes,
) {
    for function in syntax.function_bindings() {
        let scope = lexeme_range_for_bytes(lexemes, &function.scope);
        let fallback = function_body_cfg_fallback(lexemes, &scope, delimiters);
        let mut function_bindings = Vec::new();
        for parameter_bytes in &function.parameter_ranges {
            let parameter = lexeme_range_for_bytes(lexemes, parameter_bytes);
            let parameter_bindings = pattern_alias_bindings(
                body,
                lexemes,
                parameter.start,
                parameter.end,
                token_aliases,
                syntax,
            );
            if let Some((block, insertion)) = &fallback
                && let Some((_, active)) = function
                    .cfg_parameter_predicates
                    .iter()
                    .find(|(range, _)| range == parameter_bytes)
            {
                bindings.record_local_cfg_alias_fallback(
                    parameter_bindings.iter().map(|(name, _)| name),
                    CfgAliasFallbackSite {
                        block: block.clone(),
                        block_insertion: *insertion,
                        binding_insertion: *insertion,
                        active,
                        kind: CfgAliasBindingKind::Lexical,
                    },
                );
            }
            function_bindings.extend(parameter_bindings);
        }
        bindings.record(function_bindings, scope);
    }
}

pub(crate) fn function_body_cfg_fallback(
    lexemes: &[RustLexeme],
    scope: &Range<usize>,
    delimiters: &RustDelimiterMap,
) -> Option<(Range<usize>, usize)> {
    let open = first_significant_in(lexemes, scope.clone())?;
    if lexemes[open].kind != RustLexemeKind::Punctuation(b'{') {
        return None;
    }
    let close = delimiters.pairs.get(open).copied().flatten()?;
    let block = open + 1..close;
    let insertion = block_cfg_fallback_insertion(lexemes, &block, delimiters)?;
    Some((block, insertion))
}

pub(crate) fn block_cfg_fallback_insertion(
    lexemes: &[RustLexeme],
    block: &Range<usize>,
    delimiters: &RustDelimiterMap,
) -> Option<usize> {
    let mut cursor = first_significant_in(lexemes, block.clone()).unwrap_or(block.end);
    while cursor < block.end && lexemes[cursor].kind == RustLexemeKind::Punctuation(b'#') {
        let Some(bang) = first_significant_in(lexemes, cursor + 1..block.end) else {
            break;
        };
        let Some(attribute_open) = first_significant_in(lexemes, bang + 1..block.end) else {
            break;
        };
        if lexemes[bang].kind != RustLexemeKind::Punctuation(b'!')
            || lexemes[attribute_open].kind != RustLexemeKind::Punctuation(b'[')
        {
            break;
        }
        let Some(attribute_close) = delimiters.pairs.get(attribute_open).copied().flatten() else {
            break;
        };
        cursor = first_significant_in(lexemes, attribute_close + 1..block.end).unwrap_or(block.end);
    }
    lexemes
        .get(cursor)
        .map(|lexeme| lexeme.start)
        .or_else(|| lexemes.last().map(|lexeme| lexeme.end))
}

pub(crate) fn lexeme_range_for_bytes(lexemes: &[RustLexeme], bytes: &Range<usize>) -> Range<usize> {
    let start = lexemes.partition_point(|lexeme| lexeme.end <= bytes.start);
    let end = lexemes.partition_point(|lexeme| lexeme.start < bytes.end);
    start..end.max(start)
}

pub(crate) fn is_pattern_path_or_field(lexemes: &[RustLexeme], position: usize) -> bool {
    next_significant(lexemes, position).is_some_and(|next| match lexemes[next].kind {
        RustLexemeKind::Punctuation(b'(' | b'{' | b'!') => true,
        RustLexemeKind::Punctuation(b':') => next_significant(lexemes, next)
            .is_none_or(|after| lexemes[after].kind != RustLexemeKind::Punctuation(b':')),
        _ => false,
    })
}

pub(crate) fn is_token_alias_path_or_field(
    body: &str,
    lexemes: &[RustLexeme],
    position: usize,
    delimiters: &RustDelimiterMap,
) -> bool {
    next_significant(lexemes, position).is_some_and(|next| match lexemes[next].kind {
        RustLexemeKind::Punctuation(b'(') => true,
        RustLexemeKind::Punctuation(b'{') => {
            is_struct_literal_open(body, lexemes, next)
                && !is_control_flow_body_open(body, lexemes, next, delimiters)
        }
        RustLexemeKind::Punctuation(b'!') => next_significant(lexemes, next)
            .is_none_or(|after| lexemes[after].kind != RustLexemeKind::Punctuation(b'=')),
        RustLexemeKind::Punctuation(b':') => next_significant(lexemes, next)
            .is_none_or(|after| lexemes[after].kind != RustLexemeKind::Punctuation(b':')),
        _ => false,
    })
}

pub(crate) fn is_control_flow_body_open(
    body: &str,
    lexemes: &[RustLexeme],
    open: usize,
    delimiters: &RustDelimiterMap,
) -> bool {
    let scope_start = delimiters.enclosing_block(open).start.min(open);
    (scope_start..open).rev().any(|position| {
        lexemes[position].kind == RustLexemeKind::Identifier
            && matches!(
                lexeme_text(body, lexemes[position]),
                "if" | "while" | "for" | "match"
            )
            && delimiters.control_flow_body_block(body, lexemes, position + 1) == Some(open)
    })
}

pub(crate) fn is_struct_literal_open(body: &str, lexemes: &[RustLexeme], open: usize) -> bool {
    let Some(mut path_start) = previous_significant(lexemes, open) else {
        return false;
    };
    if lexemes[path_start].kind != RustLexemeKind::Identifier {
        return false;
    }
    if matches!(
        lexeme_text(body, lexemes[path_start]),
        "else" | "loop" | "unsafe" | "async" | "const"
    ) {
        return false;
    }
    while let Some(second_colon) = previous_significant(lexemes, path_start) {
        if lexemes[second_colon].kind != RustLexemeKind::Punctuation(b':') {
            break;
        }
        let Some(first_colon) = previous_significant(lexemes, second_colon) else {
            break;
        };
        if lexemes[first_colon].kind != RustLexemeKind::Punctuation(b':') {
            break;
        }
        let Some(component) = previous_significant(lexemes, first_colon) else {
            break;
        };
        if lexemes[component].kind != RustLexemeKind::Identifier {
            break;
        }
        path_start = component;
    }
    previous_significant(lexemes, path_start).is_none_or(|before| {
        lexemes[before].kind != RustLexemeKind::Identifier
            || !matches!(
                lexeme_text(body, lexemes[before]),
                "if" | "while"
                    | "for"
                    | "in"
                    | "match"
                    | "else"
                    | "loop"
                    | "unsafe"
                    | "async"
                    | "const"
                    | "fn"
                    | "struct"
                    | "enum"
                    | "union"
                    | "impl"
                    | "trait"
                    | "mod"
            )
    })
}
