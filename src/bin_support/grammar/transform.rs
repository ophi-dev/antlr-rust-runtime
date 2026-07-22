use std::collections::{BTreeMap, BTreeSet};

use super::char_support::get_char_value_from_grammar_char_literal;
use super::diagnostic::{CompilationError, Diagnostic, Severity};
use super::loader::LoadedSources;
use super::model::{
    Alternative, Authored, Block, ChannelDeclaration, Element, ElementId, ElementKind, GrammarId,
    GrammarKind, GrammarUnit, Label, Mode, ModelIdAllocator, ModelNodeId, NamedAction, Quantifier,
    Rule, RuleKind, SetElement, Terminal, TokenDeclaration, TransformId, VocabularySource,
};
use super::provenance::{MandatoryTransform, Origin, ProvenanceIndex, Tombstone};
use super::syntax::parse_grammar_unit;
use super::transform_analysis::{AnalysisInvalidation, TransformAnalysis};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SafetyClass {
    TreeAndApiPreserving,
    RecognitionPreserving,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct StructuralMetrics {
    pub(crate) rules: usize,
    pub(crate) alternatives: usize,
    pub(crate) elements: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TransformReportEntry {
    pub(crate) id: TransformId,
    pub(crate) name: &'static str,
    pub(crate) safety: SafetyClass,
    pub(crate) before: StructuralMetrics,
    pub(crate) after: StructuralMetrics,
    pub(crate) changed: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct TransformReport {
    pub(crate) entries: Vec<TransformReportEntry>,
}

pub(crate) struct TransformContext<'a> {
    pub(crate) analysis: &'a TransformAnalysis,
    pub(crate) report_only: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct TransformGrammar {
    pub(crate) units: Vec<GrammarUnit>,
    pub(crate) provenance: ProvenanceIndex,
}

pub(crate) trait GrammarTransform {
    fn name(&self) -> &'static str;
    fn safety_class(&self) -> SafetyClass;
    fn invalidates(&self) -> AnalysisInvalidation;
    fn apply(
        &self,
        input: &TransformContext<'_>,
        grammar: &mut TransformGrammar,
        report: &mut TransformReport,
    ) -> Result<bool, Diagnostic>;
}

#[derive(Default)]
pub(crate) struct TransformRegistry {
    passes: Vec<Box<dyn GrammarTransform>>,
}

impl std::fmt::Debug for TransformRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TransformRegistry")
            .field(
                "passes",
                &self
                    .passes
                    .iter()
                    .map(|pass| pass.name())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl TransformRegistry {
    pub(crate) fn push(&mut self, pass: impl GrammarTransform + 'static) {
        self.passes.push(Box::new(pass));
    }

    pub(crate) fn run(
        &self,
        grammar: &mut TransformGrammar,
        report_only: bool,
    ) -> Result<TransformReport, Diagnostic> {
        let mut report = TransformReport::default();
        let mut analysis = TransformAnalysis::compute(&grammar.units);
        for (index, pass) in self.passes.iter().enumerate() {
            let before = metrics(&grammar.units);
            let changed = pass.apply(
                &TransformContext {
                    analysis: &analysis,
                    report_only,
                },
                grammar,
                &mut report,
            )?;
            if changed && !report_only {
                validate_model(grammar)?;
                analysis.invalidate(pass.invalidates());
                analysis.recompute(&grammar.units);
            }
            report.entries.push(TransformReportEntry {
                id: TransformId::new(index as u32),
                name: pass.name(),
                safety: pass.safety_class(),
                before,
                after: metrics(&grammar.units),
                changed,
            });
        }
        Ok(report)
    }
}

fn metrics(units: &[GrammarUnit]) -> StructuralMetrics {
    let mut metrics = StructuralMetrics::default();
    for unit in units {
        metrics.rules += unit.rules.len();
        for rule in &unit.rules {
            accumulate_block(&rule.block, &mut metrics);
        }
    }
    metrics
}

fn accumulate_block(block: &Block, metrics: &mut StructuralMetrics) {
    metrics.alternatives += block.alternatives.len();
    for alternative in &block.alternatives {
        metrics.elements += alternative.elements.len();
        for element in &alternative.elements {
            if let ElementKind::Block(nested) = &element.kind {
                accumulate_block(nested, metrics);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RootOutputs {
    pub(crate) lexer: Option<GrammarId>,
    pub(crate) parser: Option<GrammarId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum IntegratedVocabularySource {
    Grammar(GrammarId),
    TokensFile(std::path::PathBuf),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct IntegratedVocabulary {
    pub(crate) consumer: GrammarId,
    pub(crate) source: IntegratedVocabularySource,
    pub(crate) declaration: Option<Authored<String>>,
}

#[derive(Clone, Debug)]
pub(crate) struct IntegratedGrammarSet {
    pub(crate) grammar: TransformGrammar,
    pub(crate) roots: BTreeMap<GrammarId, RootOutputs>,
    pub(crate) source_outputs: BTreeMap<GrammarId, RootOutputs>,
    pub(crate) vocabularies: Vec<IntegratedVocabulary>,
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) ids: ModelIdAllocator,
}

pub(crate) fn integrate_loaded(
    loaded: &LoadedSources,
) -> Result<IntegratedGrammarSet, CompilationError> {
    let mut ids = ModelIdAllocator::after_loaded_grammars(loaded.grammars.grammars.len());
    let mut provenance = ProvenanceIndex::default();
    let source_units = loaded
        .grammars
        .grammars
        .iter()
        .enumerate()
        .map(|(index, parsed)| {
            let file = loaded
                .sources
                .get(parsed.source)
                .expect("loaded grammar source is retained");
            parse_grammar_unit(
                file,
                GrammarId::new(u32::try_from(index).expect("grammar count exceeds u32")),
                &mut ids,
                &mut provenance,
            )
        })
        .collect::<Vec<_>>();

    let mut diagnostics = loaded.diagnostics.clone();
    let targets = loaded
        .grammars
        .roots
        .iter()
        .copied()
        .chain(
            loaded
                .grammars
                .vocabularies
                .iter()
                .filter_map(|edge| match edge.source {
                    VocabularySource::Grammar(grammar) => Some(grammar),
                    VocabularySource::TokensFile(_) => None,
                }),
        )
        .collect::<BTreeSet<_>>();

    let mut units = Vec::new();
    let mut roots = BTreeMap::new();
    let mut source_outputs = BTreeMap::new();
    for source in loaded
        .grammars
        .load_order
        .iter()
        .copied()
        .filter(|grammar| targets.contains(grammar))
    {
        let mut unit = source_units[source.index()].clone();
        integrate_imports(
            source,
            &mut unit,
            &source_units,
            loaded,
            ImportIntegrationContext {
                ids: &mut ids,
                provenance: &mut provenance,
                diagnostics: &mut diagnostics,
            },
        );
        reduce_unit_blocks_to_sets(&mut unit, &mut ids, &mut provenance);
        if unit.kind == GrammarKind::Combined {
            let (lexer, parser) = split_combined(unit, &mut ids, &mut provenance);
            let lexer_id = lexer.as_ref().map(|unit| unit.id);
            let parser_id = parser.id;
            if let Some(lexer) = lexer {
                units.push(lexer);
            }
            units.push(parser);
            source_outputs.insert(
                source,
                RootOutputs {
                    lexer: lexer_id,
                    parser: Some(parser_id),
                },
            );
        } else {
            let outputs = match unit.kind {
                GrammarKind::Lexer => RootOutputs {
                    lexer: Some(unit.id),
                    parser: None,
                },
                GrammarKind::Parser => RootOutputs {
                    lexer: None,
                    parser: Some(unit.id),
                },
                GrammarKind::Combined => unreachable!("combined grammar handled above"),
            };
            source_outputs.insert(source, outputs);
            units.push(unit);
        }
    }

    for root in &loaded.grammars.roots {
        if let Some(outputs) = source_outputs.get(root).copied() {
            roots.insert(*root, outputs);
        }
    }

    let mut vocabularies = Vec::new();
    for (source, outputs) in &source_outputs {
        if loaded.grammars.grammar(*source).header.kind == GrammarKind::Combined {
            if let (Some(lexer), Some(parser)) = (outputs.lexer, outputs.parser) {
                vocabularies.push(IntegratedVocabulary {
                    consumer: parser,
                    source: IntegratedVocabularySource::Grammar(lexer),
                    declaration: None,
                });
            }
        }
    }
    for edge in &loaded.grammars.vocabularies {
        let Some(consumer_outputs) = source_outputs.get(&edge.importer) else {
            continue;
        };
        let consumer = consumer_outputs
            .parser
            .or(consumer_outputs.lexer)
            .expect("integrated source has an output grammar");
        let source = match &edge.source {
            VocabularySource::Grammar(source) => {
                let producer = source_outputs
                    .get(source)
                    .and_then(|outputs| outputs.lexer)
                    .expect("loader only accepts lexer-capable source vocabulary producers");
                IntegratedVocabularySource::Grammar(producer)
            }
            VocabularySource::TokensFile(path) => {
                IntegratedVocabularySource::TokensFile(path.clone())
            }
        };
        vocabularies.push(IntegratedVocabulary {
            consumer,
            source,
            declaration: Some(edge.declaration.clone()),
        });
    }

    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
    {
        return Err(CompilationError::new(diagnostics));
    }
    let grammar = TransformGrammar { units, provenance };
    if let Err(diagnostic) = validate_model(&grammar) {
        return Err(CompilationError::new(vec![diagnostic]));
    }
    Ok(IntegratedGrammarSet {
        grammar,
        roots,
        source_outputs,
        vocabularies,
        diagnostics,
        ids,
    })
}

struct ImportIntegrationContext<'a> {
    ids: &'a mut ModelIdAllocator,
    provenance: &'a mut ProvenanceIndex,
    diagnostics: &'a mut Vec<Diagnostic>,
}

fn integrate_imports(
    root: GrammarId,
    destination: &mut GrammarUnit,
    source_units: &[GrammarUnit],
    loaded: &LoadedSources,
    context: ImportIntegrationContext<'_>,
) {
    let ImportIntegrationContext {
        ids,
        provenance,
        diagnostics,
    } = context;
    let (order, inbound_edges) = import_preorder(root, loaded);
    let mut rule_names = destination
        .rules
        .iter()
        .map(|rule| rule.name.clone())
        .collect::<BTreeSet<_>>();
    let mut channel_names = destination
        .channels
        .iter()
        .map(|channel| channel.name.value.clone())
        .collect::<BTreeSet<_>>();
    let mut action_keys = destination
        .actions
        .iter()
        .enumerate()
        .map(|(index, action)| (action_key(destination.kind, action), index))
        .collect::<BTreeMap<_, _>>();
    let mut mode_indices = destination
        .modes
        .iter()
        .enumerate()
        .map(|(index, mode)| (mode.name.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let mut default_rule_insert = destination
        .rules
        .iter()
        .position(|rule| rule.mode.is_some())
        .unwrap_or(destination.rules.len());

    for imported_id in order {
        let imported = &source_units[imported_id.index()];
        let edges = inbound_edges
            .get(&imported_id)
            .map_or(&[][..], Vec::as_slice);
        let mut cloner = ImportedCloner {
            ids,
            provenance,
            edges,
        };

        for channel in &imported.channels {
            if channel_names.insert(channel.name.value.clone()) {
                destination.channels.push(cloner.channel(channel));
            }
        }
        destination
            .tokens
            .extend(imported.tokens.iter().map(|token| cloner.token(token)));

        for action in &imported.actions {
            let key = action_key(destination.kind, action);
            if let Some(index) = action_keys.get(&key).copied() {
                let existing = &mut destination.actions[index];
                if !existing.body.is_empty() && !action.body.is_empty() {
                    existing.body.push('\n');
                }
                existing.body.push_str(&action.body);
                cloner.merge_origin(
                    ModelNodeId::Action(existing.id),
                    ModelNodeId::Action(action.id),
                );
            } else {
                let cloned = cloner.action(action);
                action_keys.insert(key, destination.actions.len());
                destination.actions.push(cloned);
            }
        }

        for mode in &imported.modes {
            let destination_mode = if let Some(index) = mode_indices.get(&mode.name).copied() {
                destination.modes[index].id
            } else {
                let cloned = cloner.mode(mode);
                let id = cloned.id;
                mode_indices.insert(mode.name.clone(), destination.modes.len());
                destination.modes.push(cloned);
                id
            };
            let mode_rules = mode.rules.iter().filter_map(|rule| {
                imported
                    .rules
                    .iter()
                    .find(|candidate| candidate.id == *rule)
            });
            for rule in mode_rules {
                if rule_names.insert(rule.name.clone()) {
                    let cloned = cloner.rule(rule, Some(destination_mode));
                    let index = mode_indices[&mode.name];
                    destination.modes[index].rules.push(cloned.id);
                    destination.rules.push(cloned);
                }
            }
        }

        for rule in &imported.rules {
            if rule.mode.is_none() && rule_names.insert(rule.name.clone()) {
                destination
                    .rules
                    .insert(default_rule_insert, cloner.rule(rule, None));
                default_rule_insert += 1;
            }
        }

        if imported.options.iter().any(|option| {
            destination.options.iter().all(|root_option| {
                root_option.name.value != option.name.value
                    || root_option.value.value != option.value.value
            })
        }) {
            let primary = imported
                .options
                .first()
                .map_or_else(|| imported.span.clone(), |option| option.name.span.clone());
            diagnostics.push(Diagnostic::warning(
                "G4T001",
                primary,
                format!(
                    "options from imported grammar {} do not alter root grammar {}",
                    imported.name, destination.name
                ),
            ));
        }
    }
}

fn import_preorder(
    root: GrammarId,
    loaded: &LoadedSources,
) -> (Vec<GrammarId>, BTreeMap<GrammarId, Vec<u32>>) {
    fn visit(
        grammar: GrammarId,
        loaded: &LoadedSources,
        seen: &mut BTreeSet<GrammarId>,
        order: &mut Vec<GrammarId>,
        inbound: &mut BTreeMap<GrammarId, Vec<u32>>,
    ) {
        for edge in loaded
            .grammars
            .imports
            .iter()
            .filter(|edge| edge.importer == grammar)
        {
            let edges = inbound.entry(edge.imported).or_default();
            if !edges.contains(&edge.id) {
                edges.push(edge.id);
            }
            if seen.insert(edge.imported) {
                order.push(edge.imported);
                visit(edge.imported, loaded, seen, order, inbound);
            }
        }
    }

    let mut seen = BTreeSet::from([root]);
    let mut order = Vec::new();
    let mut inbound = BTreeMap::new();
    visit(root, loaded, &mut seen, &mut order, &mut inbound);
    (order, inbound)
}

fn action_key(kind: GrammarKind, action: &NamedAction) -> (String, String) {
    let scope = action.scope.clone().unwrap_or_else(|| match kind {
        GrammarKind::Lexer => "lexer".to_owned(),
        GrammarKind::Parser | GrammarKind::Combined => "parser".to_owned(),
    });
    (scope, action.name.clone())
}

struct ImportedCloner<'a> {
    ids: &'a mut ModelIdAllocator,
    provenance: &'a mut ProvenanceIndex,
    edges: &'a [u32],
}

impl ImportedCloner<'_> {
    fn token(&mut self, source: &TokenDeclaration) -> TokenDeclaration {
        let mut cloned = source.clone();
        cloned.id = self.ids.token();
        self.record(ModelNodeId::Token(cloned.id), ModelNodeId::Token(source.id));
        cloned
    }

    fn channel(&mut self, source: &ChannelDeclaration) -> ChannelDeclaration {
        let mut cloned = source.clone();
        cloned.id = self.ids.channel();
        self.record(
            ModelNodeId::Channel(cloned.id),
            ModelNodeId::Channel(source.id),
        );
        cloned
    }

    fn action(&mut self, source: &NamedAction) -> NamedAction {
        let mut cloned = source.clone();
        cloned.id = self.ids.action();
        self.record(
            ModelNodeId::Action(cloned.id),
            ModelNodeId::Action(source.id),
        );
        cloned
    }

    fn mode(&mut self, source: &Mode) -> Mode {
        let mut cloned = source.clone();
        cloned.id = self.ids.mode();
        cloned.rules.clear();
        self.record(ModelNodeId::Mode(cloned.id), ModelNodeId::Mode(source.id));
        cloned
    }

    fn rule(&mut self, source: &Rule, mode: Option<super::model::ModeId>) -> Rule {
        let mut cloned = source.clone();
        cloned.id = self.ids.rule();
        cloned.mode = mode;
        cloned.actions = source
            .actions
            .iter()
            .map(|action| self.action(action))
            .collect();
        cloned.finally_action = source
            .finally_action
            .as_ref()
            .map(|action| self.action(action));
        cloned.block = self.block(&source.block);
        self.record(ModelNodeId::Rule(cloned.id), ModelNodeId::Rule(source.id));
        cloned
    }

    fn block(&mut self, source: &Block) -> Block {
        Block {
            alternatives: source
                .alternatives
                .iter()
                .map(|alternative| self.alternative(alternative))
                .collect(),
            options: source.options.clone(),
            syntax: source.syntax,
            span: source.span.clone(),
        }
    }

    fn alternative(&mut self, source: &Alternative) -> Alternative {
        let mut cloned = source.clone();
        cloned.id = self.ids.alternative();
        cloned.elements = source
            .elements
            .iter()
            .map(|element| self.element(element))
            .collect();
        self.record(
            ModelNodeId::Alternative(cloned.id),
            ModelNodeId::Alternative(source.id),
        );
        cloned
    }

    fn element(&mut self, source: &Element) -> Element {
        let mut cloned = source.clone();
        cloned.id = self.ids.element();
        cloned.label = source.label.as_ref().map(|label| self.label(label));
        cloned.kind = match &source.kind {
            ElementKind::Block(block) => ElementKind::Block(self.block(block)),
            ElementKind::Action { id, body } => {
                let cloned_id = self.ids.action();
                self.record(ModelNodeId::Action(cloned_id), ModelNodeId::Action(*id));
                ElementKind::Action {
                    id: cloned_id,
                    body: body.clone(),
                }
            }
            ElementKind::Predicate {
                id,
                body,
                fail,
                precedence,
            } => {
                let cloned_id = self.ids.predicate();
                self.record(
                    ModelNodeId::Predicate(cloned_id),
                    ModelNodeId::Predicate(*id),
                );
                ElementKind::Predicate {
                    id: cloned_id,
                    body: body.clone(),
                    fail: fail.clone(),
                    precedence: *precedence,
                }
            }
            kind => kind.clone(),
        };
        self.record(
            ModelNodeId::Element(cloned.id),
            ModelNodeId::Element(source.id),
        );
        cloned
    }

    fn label(&mut self, source: &Label) -> Label {
        let mut cloned = source.clone();
        cloned.id = self.ids.label();
        self.record(ModelNodeId::Label(cloned.id), ModelNodeId::Label(source.id));
        cloned
    }

    fn merge_origin(&mut self, destination: ModelNodeId, source: ModelNodeId) {
        let mut origins = self.provenance.origins(source).to_vec();
        origins.extend(self.edges.iter().map(|edge| Origin::Imported {
            edge: *edge,
            original: source,
        }));
        self.provenance.record_model(destination, origins);
    }

    fn record(&mut self, destination: ModelNodeId, source: ModelNodeId) {
        self.merge_origin(destination, source);
    }
}

fn reduce_unit_blocks_to_sets(
    unit: &mut GrammarUnit,
    ids: &mut ModelIdAllocator,
    provenance: &mut ProvenanceIndex,
) {
    for rule in &mut unit.rules {
        reduce_block_children(&mut rule.block, rule.kind == RuleKind::Lexer, provenance);
        reduce_top_level_block(
            &mut rule.block,
            rule.kind == RuleKind::Lexer,
            ids,
            provenance,
        );
    }
}

fn reduce_block_children(block: &mut Block, lexer: bool, provenance: &mut ProvenanceIndex) {
    for alternative in &mut block.alternatives {
        for element in &mut alternative.elements {
            let ElementKind::Block(nested) = &mut element.kind else {
                continue;
            };
            reduce_block_children(nested, lexer, provenance);
            if let Some((members, inputs)) = block_set_members(nested, lexer) {
                let replacement = ModelNodeId::Element(element.id);
                record_transform(
                    provenance,
                    replacement,
                    MandatoryTransform::BlockSetReduction,
                    &inputs,
                );
                tombstone_block(provenance, nested, &[replacement]);
                element.kind = ElementKind::Set {
                    inverted: false,
                    elements: members,
                };
            }
        }
    }
}

fn reduce_top_level_block(
    block: &mut Block,
    lexer: bool,
    ids: &mut ModelIdAllocator,
    provenance: &mut ProvenanceIndex,
) {
    let Some((members, inputs)) = block_set_members(block, lexer) else {
        return;
    };
    let new_alternative = ids.alternative();
    let new_element = ids.element();
    let old_alternatives = std::mem::take(&mut block.alternatives);
    let alternatives = old_alternatives
        .iter()
        .map(|alternative| ModelNodeId::Alternative(alternative.id))
        .collect::<Vec<_>>();
    record_transform(
        provenance,
        ModelNodeId::Alternative(new_alternative),
        MandatoryTransform::BlockSetReduction,
        &alternatives,
    );
    record_transform(
        provenance,
        ModelNodeId::Element(new_element),
        MandatoryTransform::BlockSetReduction,
        &inputs,
    );
    for alternative in &old_alternatives {
        provenance.tombstone(
            alternative.syntax,
            Tombstone {
                phase: "block-to-set",
                reason: "alternative merged into a set",
                replacements: Box::new([
                    ModelNodeId::Alternative(new_alternative),
                    ModelNodeId::Element(new_element),
                ]),
            },
        );
        for element in &alternative.elements {
            provenance.tombstone(
                element.syntax,
                Tombstone {
                    phase: "block-to-set",
                    reason: "element merged into a set",
                    replacements: Box::new([ModelNodeId::Element(new_element)]),
                },
            );
        }
    }
    block.alternatives.push(Alternative {
        id: new_alternative,
        elements: vec![Element {
            id: new_element,
            kind: ElementKind::Set {
                inverted: false,
                elements: members,
            },
            quantifier: Quantifier::One,
            label: None,
            options: Vec::new(),
            syntax: block.syntax,
            span: block.span.clone(),
            enclosing_span: block.span.clone(),
        }],
        label: None,
        options: Vec::new(),
        commands: Vec::new(),
        syntax: block.syntax,
        span: block.span.clone(),
    });
}

fn block_set_members(block: &Block, lexer: bool) -> Option<(Vec<SetElement>, Vec<ModelNodeId>)> {
    if !block.options.is_empty() || block.alternatives.len() < 2 {
        return None;
    }
    let mut members = Vec::with_capacity(block.alternatives.len());
    let mut inputs = Vec::with_capacity(block.alternatives.len());
    for alternative in &block.alternatives {
        if alternative.label.is_some()
            || !alternative.options.is_empty()
            || alternative.elements.len() != 1
        {
            return None;
        }
        let element = &alternative.elements[0];
        if element.quantifier != Quantifier::One || element.label.is_some() {
            return None;
        }
        let member = match &element.kind {
            ElementKind::Terminal(Terminal::Literal(literal))
                if !lexer || grammar_literal_code_point(literal).is_some() =>
            {
                SetElement::Terminal {
                    source: element.id,
                    value: Terminal::Literal(literal.clone()),
                    span: element.span.clone(),
                    options: element.options.clone(),
                }
            }
            ElementKind::Terminal(Terminal::Token(token)) if !lexer => SetElement::Terminal {
                source: element.id,
                value: Terminal::Token(token.clone()),
                span: element.span.clone(),
                options: element.options.clone(),
            },
            ElementKind::Range(start, stop)
                if lexer
                    && grammar_literal_code_point(start).is_some()
                    && grammar_literal_code_point(stop).is_some() =>
            {
                SetElement::Range {
                    source: element.id,
                    start: start.clone(),
                    stop: stop.clone(),
                    span: range_operator_span(&element.span, start),
                    options: element.options.clone(),
                }
            }
            _ => return None,
        };
        members.push(member);
        inputs.push(ModelNodeId::Element(element.id));
    }
    Some((members, inputs))
}

fn record_transform(
    provenance: &mut ProvenanceIndex,
    destination: ModelNodeId,
    transform: MandatoryTransform,
    inputs: &[ModelNodeId],
) {
    let mut origins = inputs
        .iter()
        .flat_map(|input| provenance.origins(*input).iter().cloned())
        .collect::<Vec<_>>();
    origins.push(Origin::MandatoryTransform {
        kind: transform,
        inputs: inputs.to_vec().into_boxed_slice(),
    });
    provenance.record_model(destination, origins);
}

fn tombstone_block(provenance: &mut ProvenanceIndex, block: &Block, replacements: &[ModelNodeId]) {
    for alternative in &block.alternatives {
        provenance.tombstone(
            alternative.syntax,
            Tombstone {
                phase: "block-to-set",
                reason: "nested alternative merged into a set",
                replacements: replacements.to_vec().into_boxed_slice(),
            },
        );
        for element in &alternative.elements {
            provenance.tombstone(
                element.syntax,
                Tombstone {
                    phase: "block-to-set",
                    reason: "nested element merged into a set",
                    replacements: replacements.to_vec().into_boxed_slice(),
                },
            );
        }
    }
}

fn grammar_literal_code_point(literal: &str) -> Option<i32> {
    let value = get_char_value_from_grammar_char_literal(Some(literal));
    (value >= 0).then_some(value)
}

fn range_operator_span(
    span: &super::frontend::SourceSpan,
    start: &str,
) -> super::frontend::SourceSpan {
    let operator_start = span
        .bytes
        .start
        .saturating_add(u32::try_from(start.len()).unwrap_or(0));
    super::frontend::SourceSpan {
        source: span.source,
        bytes: operator_start..operator_start.saturating_add(2),
    }
}

fn split_combined(
    mut combined: GrammarUnit,
    ids: &mut ModelIdAllocator,
    provenance: &mut ProvenanceIndex,
) -> (Option<GrammarUnit>, GrammarUnit) {
    let combined_id = combined.id;
    let base_name = combined.name.clone();
    let mut lexer_rules = Vec::new();
    let mut parser_rules = Vec::new();
    for rule in std::mem::take(&mut combined.rules) {
        match rule.kind {
            RuleKind::Lexer => lexer_rules.push(rule),
            RuleKind::Parser => parser_rules.push(rule),
        }
    }
    combined.rules = parser_rules;

    let literals = parser_literals(&combined.rules);
    let aliases = literal_aliases(&lexer_rules);
    let mut implicit_rules = Vec::new();
    for literal in literals {
        if aliases.contains(&literal) {
            continue;
        }
        let (original, source) = find_literal_element(&combined.rules, &literal)
            .expect("collected parser literal has an owning element");
        implicit_rules.push(implicit_literal_rule(
            ImplicitLiteralSpec {
                name: format!("T__{}", implicit_rules.len()),
                literal,
                original,
                source,
                combined: combined_id,
            },
            ids,
            provenance,
        ));
    }
    implicit_rules.append(&mut lexer_rules);

    let mut lexer_modes = std::mem::take(&mut combined.modes);
    for mode in &lexer_modes {
        add_implicit_origin(
            provenance,
            ModelNodeId::Mode(mode.id),
            combined_id,
            ModelNodeId::Mode(mode.id),
        );
    }
    for rule in &implicit_rules {
        add_rule_implicit_origins(provenance, rule, combined_id);
    }

    let lexer_actions = combined
        .actions
        .iter()
        .map(|action| clone_implicit_action(action, combined_id, ids, provenance))
        .collect::<Vec<_>>();
    combined
        .actions
        .retain(|action| action.scope.as_deref() != Some("lexer"));

    let lexer_id = ids.grammar();
    let lexer = (!implicit_rules.is_empty()).then(|| {
        let mut origins = provenance
            .origins(ModelNodeId::Grammar(combined_id))
            .to_vec();
        origins.push(Origin::ImplicitLexer {
            combined: combined_id,
            original: ModelNodeId::Grammar(combined_id),
        });
        provenance.record_model(ModelNodeId::Grammar(lexer_id), origins);
        GrammarUnit {
            id: lexer_id,
            source: combined.source,
            name: format!("{base_name}Lexer"),
            kind: GrammarKind::Lexer,
            prequels: Vec::new(),
            options: combined
                .options
                .iter()
                .filter(|option| {
                    matches!(
                        option.name.value.as_str(),
                        "contextSuperClass"
                            | "language"
                            | "accessLevel"
                            | "exportMacro"
                            | "caseInsensitive"
                    )
                })
                .cloned()
                .collect(),
            tokens: Vec::new(),
            channels: Vec::new(),
            actions: lexer_actions,
            modes: std::mem::take(&mut lexer_modes),
            rules: implicit_rules,
            syntax: combined.syntax,
            span: combined.span.clone(),
        }
    });

    combined.kind = GrammarKind::Parser;
    combined.name = format!("{base_name}Parser");
    (lexer, combined)
}

fn parser_literals(rules: &[Rule]) -> Vec<String> {
    fn visit(block: &Block, seen: &mut BTreeSet<String>, literals: &mut Vec<String>) {
        for alternative in &block.alternatives {
            for element in &alternative.elements {
                match &element.kind {
                    ElementKind::Terminal(Terminal::Literal(literal)) => {
                        if seen.insert(literal.clone()) {
                            literals.push(literal.clone());
                        }
                    }
                    ElementKind::Set { elements, .. } => {
                        for member in elements {
                            if let SetElement::Terminal {
                                value: Terminal::Literal(literal),
                                ..
                            } = member
                            {
                                if seen.insert(literal.clone()) {
                                    literals.push(literal.clone());
                                }
                            }
                        }
                    }
                    ElementKind::Block(nested) => visit(nested, seen, literals),
                    ElementKind::RuleCall(_)
                    | ElementKind::Terminal(_)
                    | ElementKind::Range(..)
                    | ElementKind::Action { .. }
                    | ElementKind::Predicate { .. }
                    | ElementKind::Epsilon => {}
                }
            }
        }
    }

    let mut seen = BTreeSet::new();
    let mut literals = Vec::new();
    for rule in rules {
        visit(&rule.block, &mut seen, &mut literals);
    }
    literals
}

fn literal_aliases(rules: &[Rule]) -> BTreeSet<String> {
    rules
        .iter()
        .filter_map(|rule| {
            let alternative = rule.block.alternatives.first()?;
            if rule.block.alternatives.len() != 1 {
                return None;
            }
            let first = alternative.elements.first()?;
            if alternative.elements[1..].iter().any(|element| {
                !matches!(
                    element.kind,
                    ElementKind::Action { .. } | ElementKind::Predicate { .. }
                )
            }) {
                return None;
            }
            match &first.kind {
                ElementKind::Terminal(Terminal::Literal(literal))
                    if first.quantifier == Quantifier::One =>
                {
                    Some(literal.clone())
                }
                _ => None,
            }
        })
        .collect()
}

fn find_literal_element<'a>(rules: &'a [Rule], literal: &str) -> Option<(&'a Element, ElementId)> {
    fn find<'a>(block: &'a Block, literal: &str) -> Option<(&'a Element, ElementId)> {
        for alternative in &block.alternatives {
            for element in &alternative.elements {
                match &element.kind {
                    ElementKind::Terminal(Terminal::Literal(candidate)) if candidate == literal => {
                        return Some((element, element.id));
                    }
                    ElementKind::Set { elements, .. } => {
                        if let Some(source) = elements.iter().find_map(|member| match member {
                            SetElement::Terminal {
                                source,
                                value: Terminal::Literal(candidate),
                                ..
                            } if candidate == literal => Some(*source),
                            _ => None,
                        }) {
                            return Some((element, source));
                        }
                    }
                    ElementKind::Block(nested) => {
                        if let Some(found) = find(nested, literal) {
                            return Some(found);
                        }
                    }
                    _ => {}
                }
            }
        }
        None
    }
    rules.iter().find_map(|rule| find(&rule.block, literal))
}

struct ImplicitLiteralSpec<'a> {
    name: String,
    literal: String,
    original: &'a Element,
    source: ElementId,
    combined: GrammarId,
}

fn implicit_literal_rule(
    spec: ImplicitLiteralSpec<'_>,
    ids: &mut ModelIdAllocator,
    provenance: &mut ProvenanceIndex,
) -> Rule {
    let ImplicitLiteralSpec {
        name,
        literal,
        original,
        source,
        combined,
    } = spec;
    let rule_id = ids.rule();
    let alternative_id = ids.alternative();
    let element_id = ids.element();
    let original_id = ModelNodeId::Element(source);
    let mut origins = provenance.origins(original_id).to_vec();
    origins.push(Origin::ImplicitLexer {
        combined,
        original: original_id,
    });
    provenance.record_model(ModelNodeId::Rule(rule_id), origins.clone());
    provenance.record_model(ModelNodeId::Alternative(alternative_id), origins.clone());
    provenance.record_model(ModelNodeId::Element(element_id), origins);
    Rule {
        id: rule_id,
        name,
        name_span: original.span.clone(),
        kind: RuleKind::Lexer,
        fragment: false,
        modifiers: Vec::new(),
        arguments: None,
        returns: None,
        locals: None,
        throws: Vec::new(),
        options: Vec::new(),
        actions: Vec::new(),
        catches: Vec::new(),
        finally_action: None,
        left_recursion: None,
        block: Block {
            alternatives: vec![Alternative {
                id: alternative_id,
                elements: vec![Element {
                    id: element_id,
                    kind: ElementKind::Terminal(Terminal::Literal(literal)),
                    quantifier: Quantifier::One,
                    label: None,
                    options: Vec::new(),
                    syntax: original.syntax,
                    span: original.span.clone(),
                    enclosing_span: original.span.clone(),
                }],
                label: None,
                options: Vec::new(),
                commands: Vec::new(),
                syntax: original.syntax,
                span: original.span.clone(),
            }],
            options: Vec::new(),
            syntax: original.syntax,
            span: original.span.clone(),
        },
        mode: None,
        case_insensitive: None,
        syntax: original.syntax,
        span: original.span.clone(),
    }
}

fn clone_implicit_action(
    source: &NamedAction,
    combined: GrammarId,
    ids: &mut ModelIdAllocator,
    provenance: &mut ProvenanceIndex,
) -> NamedAction {
    let mut cloned = source.clone();
    cloned.id = ids.action();
    let source_id = ModelNodeId::Action(source.id);
    let mut origins = provenance.origins(source_id).to_vec();
    origins.push(Origin::ImplicitLexer {
        combined,
        original: source_id,
    });
    provenance.record_model(ModelNodeId::Action(cloned.id), origins);
    cloned
}

fn add_rule_implicit_origins(provenance: &mut ProvenanceIndex, rule: &Rule, combined: GrammarId) {
    add_implicit_origin(
        provenance,
        ModelNodeId::Rule(rule.id),
        combined,
        ModelNodeId::Rule(rule.id),
    );
    for action in &rule.actions {
        add_implicit_origin(
            provenance,
            ModelNodeId::Action(action.id),
            combined,
            ModelNodeId::Action(action.id),
        );
    }
    if let Some(action) = &rule.finally_action {
        add_implicit_origin(
            provenance,
            ModelNodeId::Action(action.id),
            combined,
            ModelNodeId::Action(action.id),
        );
    }
    add_block_implicit_origins(provenance, &rule.block, combined);
}

fn add_block_implicit_origins(
    provenance: &mut ProvenanceIndex,
    block: &Block,
    combined: GrammarId,
) {
    for alternative in &block.alternatives {
        add_implicit_origin(
            provenance,
            ModelNodeId::Alternative(alternative.id),
            combined,
            ModelNodeId::Alternative(alternative.id),
        );
        for element in &alternative.elements {
            add_implicit_origin(
                provenance,
                ModelNodeId::Element(element.id),
                combined,
                ModelNodeId::Element(element.id),
            );
            if let Some(label) = &element.label {
                add_implicit_origin(
                    provenance,
                    ModelNodeId::Label(label.id),
                    combined,
                    ModelNodeId::Label(label.id),
                );
            }
            match &element.kind {
                ElementKind::Action { id, .. } => add_implicit_origin(
                    provenance,
                    ModelNodeId::Action(*id),
                    combined,
                    ModelNodeId::Action(*id),
                ),
                ElementKind::Predicate { id, .. } => add_implicit_origin(
                    provenance,
                    ModelNodeId::Predicate(*id),
                    combined,
                    ModelNodeId::Predicate(*id),
                ),
                ElementKind::Block(nested) => {
                    add_block_implicit_origins(provenance, nested, combined);
                }
                _ => {}
            }
        }
    }
}

fn add_implicit_origin(
    provenance: &mut ProvenanceIndex,
    destination: ModelNodeId,
    combined: GrammarId,
    original: ModelNodeId,
) {
    provenance.record_model(destination, [Origin::ImplicitLexer { combined, original }]);
}

pub(crate) fn validate_model(grammar: &TransformGrammar) -> Result<(), Diagnostic> {
    let mut nodes = BTreeSet::new();
    for unit in &grammar.units {
        insert_model_node(&mut nodes, ModelNodeId::Grammar(unit.id), unit.span.clone())?;
        for token in &unit.tokens {
            insert_model_node(
                &mut nodes,
                ModelNodeId::Token(token.id),
                token.name.span.clone(),
            )?;
        }
        for channel in &unit.channels {
            insert_model_node(
                &mut nodes,
                ModelNodeId::Channel(channel.id),
                channel.name.span.clone(),
            )?;
        }
        for action in &unit.actions {
            insert_model_node(
                &mut nodes,
                ModelNodeId::Action(action.id),
                action.span.clone(),
            )?;
        }
        let modes = unit
            .modes
            .iter()
            .map(|mode| mode.id)
            .collect::<BTreeSet<_>>();
        let rules = unit
            .rules
            .iter()
            .map(|rule| rule.id)
            .collect::<BTreeSet<_>>();
        for mode in &unit.modes {
            insert_model_node(&mut nodes, ModelNodeId::Mode(mode.id), mode.span.clone())?;
            if let Some(missing) = mode.rules.iter().find(|rule| !rules.contains(rule)) {
                return Err(Diagnostic::error(
                    "G4T902",
                    mode.span.clone(),
                    format!(
                        "mode {} references missing rule ID {}",
                        mode.name,
                        missing.index()
                    ),
                ));
            }
        }
        for rule in &unit.rules {
            insert_model_node(&mut nodes, ModelNodeId::Rule(rule.id), rule.span.clone())?;
            if rule.mode.is_some_and(|mode| !modes.contains(&mode)) {
                return Err(Diagnostic::error(
                    "G4T903",
                    rule.span.clone(),
                    format!("rule {} references a missing mode", rule.name),
                ));
            }
            for action in &rule.actions {
                insert_model_node(
                    &mut nodes,
                    ModelNodeId::Action(action.id),
                    action.span.clone(),
                )?;
            }
            if let Some(action) = &rule.finally_action {
                insert_model_node(
                    &mut nodes,
                    ModelNodeId::Action(action.id),
                    action.span.clone(),
                )?;
            }
            collect_block_nodes(&rule.block, &mut nodes)?;
        }
    }
    if let Some(node) = nodes
        .iter()
        .find(|node| grammar.provenance.origins(**node).is_empty())
    {
        let span = grammar.units.first().map_or_else(
            || super::frontend::SourceSpan::empty(super::frontend::SourceId::new(0)),
            |unit| unit.span.clone(),
        );
        return Err(Diagnostic::error(
            "G4T904",
            span,
            format!("model node {node:?} has no provenance"),
        ));
    }
    Ok(())
}

fn collect_block_nodes(block: &Block, nodes: &mut BTreeSet<ModelNodeId>) -> Result<(), Diagnostic> {
    for alternative in &block.alternatives {
        insert_model_node(
            nodes,
            ModelNodeId::Alternative(alternative.id),
            alternative.span.clone(),
        )?;
        for element in &alternative.elements {
            insert_model_node(
                nodes,
                ModelNodeId::Element(element.id),
                element.enclosing_span.clone(),
            )?;
            if let Some(label) = &element.label {
                insert_model_node(nodes, ModelNodeId::Label(label.id), label.span.clone())?;
            }
            match &element.kind {
                ElementKind::Action { id, .. } => {
                    insert_model_node(nodes, ModelNodeId::Action(*id), element.span.clone())?;
                }
                ElementKind::Predicate { id, .. } => {
                    insert_model_node(nodes, ModelNodeId::Predicate(*id), element.span.clone())?;
                }
                ElementKind::Block(nested) => collect_block_nodes(nested, nodes)?,
                _ => {}
            }
        }
    }
    Ok(())
}

fn insert_model_node(
    nodes: &mut BTreeSet<ModelNodeId>,
    node: ModelNodeId,
    span: super::frontend::SourceSpan,
) -> Result<(), Diagnostic> {
    if nodes.insert(node) {
        Ok(())
    } else {
        Err(Diagnostic::error(
            "G4T901",
            span,
            format!("model node ID {node:?} is used more than once"),
        ))
    }
}

pub(crate) fn render_unmodified_sources(
    sources: &super::source::SourceSet,
) -> BTreeMap<super::frontend::SourceId, String> {
    sources
        .iter()
        .map(|source| (source.id(), source.text().to_owned()))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write as _;
    use std::path::PathBuf;

    use super::*;
    use crate::grammar::loader::{LoadOptions, load};

    #[test]
    fn integrates_diamond_imports_reduces_sets_and_splits_combined_grammar() {
        let fixture = Fixture::new("integrate");
        fixture.write(
            "Root.g4",
            r#"
grammar Root;
import Left, Right;
@members { root(); }
root : 'if' | ID;
ID : 'id';
WS : ' ' -> skip;
"#,
        );
        fixture.write(
            "Left.g4",
            r#"
parser grammar Left;
import Shared;
@members { left(); }
root : 'override';
left : 'left';
"#,
        );
        fixture.write(
            "Right.g4",
            r#"
parser grammar Right;
import Shared;
@members { right(); }
right : 'right';
"#,
        );
        fixture.write("Shared.g4", "parser grammar Shared; shared : 'shared';");

        let loaded = load(LoadOptions {
            roots: vec![fixture.path("Root.g4")],
            library_directories: Vec::new(),
        })
        .expect("fixture should load");
        let integrated = integrate_loaded(&loaded).expect("fixture should integrate");
        let outputs = integrated.roots[&GrammarId::new(0)];
        let lexer = integrated
            .grammar
            .units
            .iter()
            .find(|unit| Some(unit.id) == outputs.lexer)
            .expect("implicit lexer");
        let parser = integrated
            .grammar
            .units
            .iter()
            .find(|unit| Some(unit.id) == outputs.parser)
            .expect("combined parser");

        assert_eq!(lexer.name, "RootLexer");
        assert_eq!(parser.name, "RootParser");
        assert_eq!(
            parser
                .rules
                .iter()
                .map(|rule| rule.name.as_str())
                .collect::<Vec<_>>(),
            ["root", "left", "shared", "right"]
        );
        assert!(matches!(
            parser.rules[0].block.alternatives[0].elements[0].kind,
            ElementKind::Set {
                inverted: false,
                ..
            }
        ));
        assert_eq!(parser.actions[0].body, " root(); \n left(); \n right(); ");
        assert_eq!(
            lexer
                .rules
                .iter()
                .map(|rule| rule.name.as_str())
                .collect::<Vec<_>>(),
            ["T__0", "T__1", "T__2", "T__3", "ID", "WS"]
        );

        let shared = parser
            .rules
            .iter()
            .find(|rule| rule.name == "shared")
            .expect("shared imported rule");
        let imported_edges = integrated
            .grammar
            .provenance
            .origins(ModelNodeId::Rule(shared.id))
            .iter()
            .filter_map(|origin| match origin {
                Origin::Imported { edge, .. } => Some(*edge),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(imported_edges.len(), 2);
        validate_model(&integrated.grammar).expect("integrated model should be valid");
    }

    #[test]
    fn no_op_renderer_is_byte_identical() {
        let fixture = Fixture::new("render");
        let text = "parser grammar A;\n// retained trivia\na : 'x' ;\n";
        fixture.write("A.g4", text);
        let loaded = load(LoadOptions {
            roots: vec![fixture.path("A.g4")],
            library_directories: Vec::new(),
        })
        .expect("fixture should load");
        let rendered = render_unmodified_sources(&loaded.sources);
        let source = loaded
            .sources
            .iter()
            .next()
            .expect("fixture contains one source");
        assert_eq!(rendered[&source.id()], text);
    }

    #[test]
    fn integrates_dependency_only_source_vocabulary_before_its_consumer() {
        let fixture = Fixture::new("source-vocabulary");
        fixture.write(
            "Root.g4",
            "parser grammar Root; options { tokenVocab=Lex; } root : ID;",
        );
        fixture.write("Lex.g4", "lexer grammar Lex; ID : [a-z]+;");

        let loaded = load(LoadOptions {
            roots: vec![fixture.path("Root.g4")],
            library_directories: Vec::new(),
        })
        .expect("source-backed vocabulary should load");
        let integrated =
            integrate_loaded(&loaded).expect("source-backed vocabulary should integrate");

        assert_eq!(
            integrated
                .grammar
                .units
                .iter()
                .map(|unit| unit.name.as_str())
                .collect::<Vec<_>>(),
            ["Lex", "Root"]
        );
        assert_eq!(
            integrated.source_outputs[&GrammarId::new(1)].lexer,
            Some(GrammarId::new(1))
        );
        assert_eq!(
            integrated.vocabularies,
            [IntegratedVocabulary {
                consumer: GrammarId::new(0),
                source: IntegratedVocabularySource::Grammar(GrammarId::new(1)),
                declaration: loaded.grammars.vocabularies[0].declaration.clone().into(),
            }]
        );
    }

    #[test]
    fn imported_lexer_rules_precede_root_mode_rules() {
        let fixture = Fixture::new("lexer-import-modes");
        fixture.write(
            "Root.g4",
            "lexer grammar Root; import Base; A: 'a'; mode M; B: 'b';",
        );
        fixture.write(
            "Base.g4",
            "lexer grammar Base; fragment X: 'x'; fragment Y: 'y';",
        );

        let loaded = load(LoadOptions {
            roots: vec![fixture.path("Root.g4")],
            library_directories: Vec::new(),
        })
        .expect("fixture should load");
        let integrated = integrate_loaded(&loaded).expect("fixture should integrate");
        assert_eq!(
            integrated.grammar.units[0]
                .rules
                .iter()
                .map(|rule| rule.name.as_str())
                .collect::<Vec<_>>(),
            ["A", "X", "Y", "B"]
        );
    }

    #[test]
    fn mutation_recomputes_declared_analysis_before_the_next_pass() {
        struct Rename;
        impl GrammarTransform for Rename {
            fn name(&self) -> &'static str {
                "rename"
            }

            fn safety_class(&self) -> SafetyClass {
                SafetyClass::TreeAndApiPreserving
            }

            fn invalidates(&self) -> AnalysisInvalidation {
                AnalysisInvalidation::NAMES.union(AnalysisInvalidation::CALLS)
            }

            fn apply(
                &self,
                _input: &TransformContext<'_>,
                grammar: &mut TransformGrammar,
                _report: &mut TransformReport,
            ) -> Result<bool, Diagnostic> {
                grammar.units[0].rules[0].name = "renamed".to_owned();
                Ok(true)
            }
        }

        struct Observe;
        impl GrammarTransform for Observe {
            fn name(&self) -> &'static str {
                "observe"
            }

            fn safety_class(&self) -> SafetyClass {
                SafetyClass::TreeAndApiPreserving
            }

            fn invalidates(&self) -> AnalysisInvalidation {
                AnalysisInvalidation::default()
            }

            fn apply(
                &self,
                input: &TransformContext<'_>,
                _grammar: &mut TransformGrammar,
                _report: &mut TransformReport,
            ) -> Result<bool, Diagnostic> {
                assert!(input.analysis.rules_by_name.contains_key("renamed"));
                Ok(false)
            }
        }

        let fixture = Fixture::new("invalidate");
        fixture.write("A.g4", "parser grammar A; a : 'x';");
        let loaded = load(LoadOptions {
            roots: vec![fixture.path("A.g4")],
            library_directories: Vec::new(),
        })
        .expect("fixture should load");
        let mut integrated = integrate_loaded(&loaded).expect("fixture should integrate");
        let mut registry = TransformRegistry::default();
        registry.push(Rename);
        registry.push(Observe);
        let report = registry
            .run(&mut integrated.grammar, false)
            .expect("passes should run");
        assert_eq!(report.entries.len(), 2);
        assert!(report.entries[0].changed);
    }

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "antlr-rust-phase-b-transform-{name}-{}-{serial}",
                std::process::id()
            ));
            fs::create_dir_all(&root).expect("fixture directory");
            Self { root }
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.root.join(relative)
        }

        fn write(&self, relative: &str, text: &str) {
            let path = self.path(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("fixture parent");
            }
            let mut file = fs::File::create(&path).expect("fixture file");
            file.write_all(text.as_bytes()).expect("fixture contents");
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
