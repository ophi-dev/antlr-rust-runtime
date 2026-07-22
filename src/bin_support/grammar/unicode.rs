use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Mutex, OnceLock, PoisonError};

use icu_casemap::CaseMapper;
use icu_properties::props;
use icu_properties::{
    CodePointMapData, CodePointSetData, CodePointSetDataBorrowed, EmojiSetData, PropertyParser,
};
use intl::unicode::{
    IndicPositionalCategory, IsNormalized, canonical_combining_class, indic_positional_category,
    nfd, quick_check_nfc, quick_check_nfd, quick_check_nfkc, quick_check_nfkd,
};

const MAX_CODE_POINT: i32 = 0x10_ffff;

type CachedRanges = Option<&'static [i32]>;
type RangesByU8 = HashMap<u8, Vec<i32>>;
type DecompositionCccRanges = (RangesByU8, RangesByU8);

#[derive(Default)]
struct PropertyCache {
    by_name: HashMap<String, CachedRanges>,
    by_ranges: HashMap<u64, Vec<&'static [i32]>>,
}

static PROPERTY_CACHE: OnceLock<Mutex<PropertyCache>> = OnceLock::new();
static BLOCKS: OnceLock<HashMap<String, Vec<i32>>> = OnceLock::new();
static INDIC_POSITIONAL_RANGES: OnceLock<HashMap<&'static str, Vec<i32>>> = OnceLock::new();
static NORMALIZATION_QUICK_CHECK_RANGES: OnceLock<HashMap<(&'static str, u8), Vec<i32>>> =
    OnceLock::new();
static DECOMPOSITION_CCC_RANGES: OnceLock<DecompositionCccRanges> = OnceLock::new();
static DECOMPOSITION_TYPE_RANGES: OnceLock<RangesByU8> = OnceLock::new();

pub(crate) fn property_ranges(name: &str) -> Option<&'static [i32]> {
    let normalized = normalize_property_name(name);
    let cache = PROPERTY_CACHE.get_or_init(|| Mutex::new(PropertyCache::default()));
    let mut cache = cache.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(ranges) = cache.by_name.get(&normalized) {
        return *ranges;
    }

    let ranges =
        compute_property_ranges(&normalized).map(|ranges| cache_ranges(&mut cache, ranges));
    cache.by_name.insert(normalized, ranges);
    ranges
}

fn cache_ranges(cache: &mut PropertyCache, ranges: Vec<i32>) -> &'static [i32] {
    let mut hasher = DefaultHasher::new();
    ranges.hash(&mut hasher);
    let fingerprint = hasher.finish();
    if let Some(existing) = cache.by_ranges.get(&fingerprint).and_then(|candidates| {
        candidates
            .iter()
            .find(|candidate| **candidate == ranges.as_slice())
    }) {
        return existing;
    }

    let leaked: &'static mut [i32] = Box::leak(ranges.into_boxed_slice());
    let ranges = &*leaked;
    cache.by_ranges.entry(fingerprint).or_default().push(ranges);
    ranges
}

pub(crate) fn simple_lowercase(code_point: i32) -> i32 {
    let Some(character) = scalar_value(code_point) else {
        return code_point;
    };
    CaseMapper::new().simple_lowercase(character) as i32
}

pub(crate) fn simple_uppercase(code_point: i32) -> i32 {
    let Some(character) = scalar_value(code_point) else {
        return code_point;
    };
    CaseMapper::new().simple_uppercase(character) as i32
}

fn scalar_value(code_point: i32) -> Option<char> {
    u32::try_from(code_point).ok().and_then(char::from_u32)
}

fn compute_property_ranges(name: &str) -> Option<Vec<i32>> {
    if name == "space" {
        return binary_ranges::<props::WhiteSpace>("wspace");
    }
    if name == "control" {
        return general_category_ranges("c");
    }
    if name == "extendedpictographic" || name == "ep" {
        return Some(ANTLR_EXTENDED_PICTOGRAPHIC.to_vec());
    }
    if let Some(ranges) = emoji_string_property_ranges(name) {
        return Some(ranges);
    }
    if name == "emojirk" {
        return Some(emoji_recommended_for_keyboard_ranges());
    }
    if name == "emojinrk" {
        let emoji = ranges_from_set(CodePointSetData::new::<props::Emoji>());
        return Some(subtract_ranges(
            &emoji,
            &emoji_recommended_for_keyboard_ranges(),
        ));
    }

    if let Some((property, value)) = name.split_once('=') {
        return explicit_property_ranges(property, value);
    }

    general_category_ranges(name)
        .or_else(|| script_ranges(name))
        .or_else(|| binary_property_ranges(name))
        .or_else(|| {
            name.strip_prefix("in")
                .filter(|block| !block.is_empty())
                .and_then(block_ranges)
        })
}

fn explicit_property_ranges(property: &str, value: &str) -> Option<Vec<i32>> {
    if matches_name(property, b"General_Category", b"gc") {
        return general_category_ranges(value);
    }
    if matches_name(property, b"Script", b"sc") {
        return script_ranges(value);
    }
    if matches_name(property, b"Block", b"blk") {
        return block_ranges(value);
    }
    if property == "emojipresentation" {
        return emoji_presentation_ranges(value);
    }
    if matches_name(property, b"Bidi_Paired_Bracket_Type", b"bpt") {
        return bidi_paired_bracket_type_ranges(value);
    }
    if matches_name(property, b"Indic_Positional_Category", b"inpc") {
        return indic_positional_category_ranges(value);
    }
    if matches_name(property, b"Lead_Canonical_Combining_Class", b"lccc") {
        return decomposition_ccc_ranges(value, true);
    }
    if matches_name(property, b"Trail_Canonical_Combining_Class", b"tccc") {
        return decomposition_ccc_ranges(value, false);
    }
    if matches_name(property, b"Decomposition_Type", b"dt") {
        return decomposition_type_ranges(value);
    }
    if let Some(form) = normalization_form(property) {
        return normalization_quick_check_ranges(form, value);
    }
    enumerated_property_ranges(property, value)
}

fn general_category_ranges(name: &str) -> Option<Vec<i32>> {
    let group = PropertyParser::<props::GeneralCategoryGroup>::new().get_loose(name)?;
    let ranges = CodePointMapData::<props::GeneralCategory>::new().get_set_for_value_group(group);
    Some(ranges_from_set(ranges.as_borrowed()))
}

fn script_ranges(name: &str) -> Option<Vec<i32>> {
    let script = PropertyParser::<props::Script>::new().get_loose(name)?;
    Some(ranges_from_iter(
        CodePointMapData::<props::Script>::new().iter_ranges_for_value(script),
    ))
}

macro_rules! try_enumerated_property {
    ($property:expr, $value:expr, $type:ty) => {
        if matches_name(
            $property,
            <$type as props::EnumeratedProperty>::NAME,
            <$type as props::EnumeratedProperty>::SHORT_NAME,
        ) {
            let parsed = PropertyParser::<$type>::new().get_loose($value)?;
            return Some(ranges_from_iter(
                CodePointMapData::<$type>::new().iter_ranges_for_value(parsed),
            ));
        }
    };
}

fn enumerated_property_ranges(property: &str, value: &str) -> Option<Vec<i32>> {
    if matches_name(property, b"Canonical_Combining_Class", b"ccc") && value == "null" {
        return Some(Vec::new());
    }
    try_enumerated_property!(property, value, props::BidiClass);
    try_enumerated_property!(property, value, props::CanonicalCombiningClass);
    try_enumerated_property!(property, value, props::EastAsianWidth);
    try_enumerated_property!(property, value, props::GraphemeClusterBreak);
    try_enumerated_property!(property, value, props::HangulSyllableType);
    try_enumerated_property!(property, value, props::IndicConjunctBreak);
    try_enumerated_property!(property, value, props::IndicSyllabicCategory);
    try_enumerated_property!(property, value, props::JoiningGroup);
    try_enumerated_property!(property, value, props::JoiningType);
    try_enumerated_property!(property, value, props::LineBreak);
    try_enumerated_property!(property, value, props::NumericType);
    try_enumerated_property!(property, value, props::SentenceBreak);
    try_enumerated_property!(property, value, props::VerticalOrientation);
    try_enumerated_property!(property, value, props::WordBreak);
    None
}

macro_rules! try_binary_properties {
    ($name:expr, $($property:ty),+ $(,)?) => {
        $(
            if let Some(ranges) = binary_ranges::<$property>($name) {
                return Some(ranges);
            }
        )+
    };
}

fn binary_property_ranges(name: &str) -> Option<Vec<i32>> {
    try_binary_properties!(
        name,
        props::AsciiHexDigit,
        props::Alnum,
        props::Alphabetic,
        props::BidiControl,
        props::BidiMirrored,
        props::Blank,
        props::Cased,
        props::CaseIgnorable,
        props::FullCompositionExclusion,
        props::ChangesWhenCasefolded,
        props::ChangesWhenCasemapped,
        props::ChangesWhenNfkcCasefolded,
        props::ChangesWhenLowercased,
        props::ChangesWhenTitlecased,
        props::ChangesWhenUppercased,
        props::Dash,
        props::Deprecated,
        props::DefaultIgnorableCodePoint,
        props::Diacritic,
        props::EmojiModifierBase,
        props::EmojiComponent,
        props::EmojiModifier,
        props::Emoji,
        props::EmojiPresentation,
        props::ExtendedPictographic,
        props::Extender,
        props::Graph,
        props::GraphemeBase,
        props::GraphemeExtend,
        props::GraphemeLink,
        props::HexDigit,
        props::Hyphen,
        props::IdCompatMathContinue,
        props::IdCompatMathStart,
        props::IdContinue,
        props::Ideographic,
        props::IdStart,
        props::IdsBinaryOperator,
        props::IdsTrinaryOperator,
        props::IdsUnaryOperator,
        props::JoinControl,
        props::LogicalOrderException,
        props::Lowercase,
        props::Math,
        props::ModifierCombiningMark,
        props::NoncharacterCodePoint,
        props::NfcInert,
        props::NfdInert,
        props::NfkcInert,
        props::NfkdInert,
        props::PatternSyntax,
        props::PatternWhiteSpace,
        props::PrependedConcatenationMark,
        props::Print,
        props::QuotationMark,
        props::Radical,
        props::RegionalIndicator,
        props::SoftDotted,
        props::SegmentStarter,
        props::CaseSensitive,
        props::SentenceTerminal,
        props::TerminalPunctuation,
        props::UnifiedIdeograph,
        props::Uppercase,
        props::VariationSelector,
        props::WhiteSpace,
        props::Xdigit,
        props::XidContinue,
        props::XidStart,
    );
    None
}

fn binary_ranges<Property>(name: &str) -> Option<Vec<i32>>
where
    Property: props::BinaryProperty,
{
    matches_name(name, Property::NAME, Property::SHORT_NAME)
        .then(|| ranges_from_set(CodePointSetData::new::<Property>()))
}

fn emoji_presentation_ranges(value: &str) -> Option<Vec<i32>> {
    let emoji = || ranges_from_set(CodePointSetData::new::<props::Emoji>());
    let emoji_default = || ranges_from_set(CodePointSetData::new::<props::EmojiPresentation>());
    match value {
        "emojidefault" => Some(emoji_default()),
        "textdefault" => Some(subtract_ranges(&emoji(), &emoji_default())),
        "text" => Some(subtract_ranges(&[0, MAX_CODE_POINT], &emoji())),
        _ => None,
    }
}

fn emoji_string_property_ranges(name: &str) -> Option<Vec<i32>> {
    match name {
        "basicemoji" | "rgiemoji" => {
            let basic_emoji = EmojiSetData::new::<props::BasicEmoji>();
            Some(ranges_matching(|code_point| {
                basic_emoji.contains32(code_point)
            }))
        }
        "emojikeycapsequence"
        | "rgiemojiflagsequence"
        | "rgiemojimodifiersequence"
        | "rgiemojitagsequence"
        | "rgiemojizwjsequence" => Some(Vec::new()),
        _ => None,
    }
}

fn emoji_recommended_for_keyboard_ranges() -> Vec<i32> {
    let regional_indicators = CodePointSetData::new::<props::RegionalIndicator>();
    ranges_matching(|code_point| {
        regional_indicators.contains32(code_point)
            || matches!(
                code_point,
                0x23 | 0x2a | 0x30..=0x39 | 0x00a9 | 0x00ae | 0x2122 | 0x3030 | 0x303d
            )
    })
}

fn bidi_paired_bracket_type_ranges(value: &str) -> Option<Vec<i32>> {
    let ranges_for = |bracket_type| {
        ranges_from_iter(
            CodePointMapData::<props::BidiMirroringGlyph>::new()
                .iter_ranges()
                .filter(move |range| range.value.paired_bracket_type == bracket_type)
                .map(|range| range.range),
        )
    };
    match value {
        "o" | "open" => Some(ranges_for(props::BidiPairedBracketType::Open)),
        "c" | "close" => Some(ranges_for(props::BidiPairedBracketType::Close)),
        "n" | "none" => {
            let brackets = union_ranges(
                &ranges_for(props::BidiPairedBracketType::Open),
                &ranges_for(props::BidiPairedBracketType::Close),
            );
            Some(subtract_ranges(&[0, MAX_CODE_POINT], &brackets))
        }
        _ => None,
    }
}

// ANTLR 4.13.2 exposes this historical TR35 set under the long
// Extended_Pictographic name. ICU's current ExtPict property remains available
// separately through the standard short name.
#[rustfmt::skip]
#[allow(clippy::unreadable_literal)]
const ANTLR_EXTENDED_PICTOGRAPHIC: &[i32] = &[
    9096, 9096, 9733, 9733, 9735, 9741, 9743, 9744, 9746, 9746, 9750, 9751,
    9753, 9756, 9758, 9759, 9761, 9761, 9764, 9765, 9767, 9769, 9771, 9773,
    9776, 9783, 9787, 9799, 9812, 9823, 9825, 9826, 9828, 9828, 9831, 9831,
    9833, 9850, 9852, 9854, 9856, 9873, 9877, 9877, 9880, 9880, 9882, 9882,
    9885, 9887, 9890, 9897, 9900, 9903, 9906, 9916, 9919, 9923, 9926, 9927,
    9929, 9933, 9936, 9936, 9938, 9938, 9941, 9960, 9963, 9967, 9974, 9974,
    9979, 9980, 9982, 9985, 9987, 9988, 9998, 9998, 10000, 10001, 10085,
    10087, 126976, 126979, 126981, 127231, 127245, 127247, 127279, 127279,
    127340, 127343, 127405, 127461, 127491, 127503, 127548, 127551, 127561,
    127567, 127570, 127743, 127778, 127779, 127892, 127893, 127896, 127896,
    127900, 127901, 127985, 127986, 127990, 127990, 128254, 128254, 128318,
    128328, 128335, 128335, 128360, 128366, 128369, 128370, 128379, 128390,
    128392, 128393, 128398, 128399, 128401, 128404, 128407, 128419, 128422,
    128423, 128425, 128432, 128435, 128443, 128445, 128449, 128453, 128464,
    128468, 128475, 128479, 128480, 128482, 128482, 128484, 128487, 128489,
    128494, 128496, 128498, 128500, 128505, 128710, 128714, 128723, 128735,
    128742, 128744, 128746, 128746, 128749, 128751, 128753, 128754, 128759,
    128767, 128884, 128895, 128981, 129023, 129036, 129039, 129096, 129103,
    129114, 129119, 129160, 129167, 129198, 129295, 129311, 129311, 129320,
    129327, 129329, 129330, 129343, 129343, 129356, 129359, 129375, 129407,
    129426, 129471, 129473, 131069,
];

fn block_ranges(name: &str) -> Option<Vec<i32>> {
    unicode_blocks().get(name).cloned()
}

fn unicode_blocks() -> &'static HashMap<String, Vec<i32>> {
    BLOCKS.get_or_init(|| {
        let mut blocks = HashMap::new();
        for (name, ranges) in ranges_by_value(unicode_block_name) {
            blocks.insert(normalize_property_name(name), ranges);
        }
        for &(alias, canonical) in BLOCK_ALIASES {
            let ranges = blocks
                .get(canonical)
                .unwrap_or_else(|| panic!("Unicode block alias references {canonical}"))
                .clone();
            blocks.insert(alias.to_owned(), ranges);
        }
        blocks
    })
}

fn unicode_block_name(code_point: u32) -> &'static str {
    match code_point {
        0xd800..=0xdb7f => "High Surrogates",
        0xdb80..=0xdbff => "High Private Use Surrogates",
        0xdc00..=0xdfff => "Low Surrogates",
        _ => char::from_u32(code_point).map_or("No_Block", intl::unicode::block),
    }
}

// Short block aliases whose loose form differs from the Unicode 17 long name.
// Generated from UCD PropertyValueAliases.txt; aliases identical after loose
// normalization need no entry.
const BLOCK_ALIASES: &[(&str, &str)] = &[
    ("alchemical", "alchemicalsymbols"),
    ("alphabeticpf", "alphabeticpresentationforms"),
    ("ancientgreekmusic", "ancientgreekmusicalnotation"),
    ("arabicexta", "arabicextendeda"),
    ("arabicextb", "arabicextendedb"),
    ("arabicextc", "arabicextendedc"),
    ("arabicmath", "arabicmathematicalalphabeticsymbols"),
    ("arabicpfa", "arabicpresentationformsa"),
    ("arabicpfb", "arabicpresentationformsb"),
    ("arabicsup", "arabicsupplement"),
    ("ascii", "basiclatin"),
    ("bamumsup", "bamumsupplement"),
    ("bopomofoext", "bopomofoextended"),
    ("braille", "braillepatterns"),
    ("byzantinemusic", "byzantinemusicalsymbols"),
    ("cherokeesup", "cherokeesupplement"),
    ("cjk", "cjkunifiedideographs"),
    ("cjkcompat", "cjkcompatibility"),
    ("cjkcompatforms", "cjkcompatibilityforms"),
    ("cjkcompatideographs", "cjkcompatibilityideographs"),
    (
        "cjkcompatideographssup",
        "cjkcompatibilityideographssupplement",
    ),
    ("cjkexta", "cjkunifiedideographsextensiona"),
    ("cjkextb", "cjkunifiedideographsextensionb"),
    ("cjkextc", "cjkunifiedideographsextensionc"),
    ("cjkextd", "cjkunifiedideographsextensiond"),
    ("cjkexte", "cjkunifiedideographsextensione"),
    ("cjkextf", "cjkunifiedideographsextensionf"),
    ("cjkextg", "cjkunifiedideographsextensiong"),
    ("cjkexth", "cjkunifiedideographsextensionh"),
    ("cjkexti", "cjkunifiedideographsextensioni"),
    ("cjkextj", "cjkunifiedideographsextensionj"),
    ("cjkradicalssup", "cjkradicalssupplement"),
    ("cjksymbols", "cjksymbolsandpunctuation"),
    ("compatjamo", "hangulcompatibilityjamo"),
    ("countingrod", "countingrodnumerals"),
    ("cuneiformnumbers", "cuneiformnumbersandpunctuation"),
    ("cyrillicexta", "cyrillicextendeda"),
    ("cyrillicextb", "cyrillicextendedb"),
    ("cyrillicextc", "cyrillicextendedc"),
    ("cyrillicextd", "cyrillicextendedd"),
    ("cyrillicsup", "cyrillicsupplement"),
    ("devanagariext", "devanagariextended"),
    ("devanagariexta", "devanagariextendeda"),
    ("diacriticals", "combiningdiacriticalmarks"),
    ("diacriticalsext", "combiningdiacriticalmarksextended"),
    (
        "diacriticalsforsymbols",
        "combiningdiacriticalmarksforsymbols",
    ),
    ("diacriticalssup", "combiningdiacriticalmarkssupplement"),
    ("domino", "dominotiles"),
    ("egyptianhieroglyphsexta", "egyptianhieroglyphsextendeda"),
    ("enclosedalphanum", "enclosedalphanumerics"),
    ("enclosedalphanumsup", "enclosedalphanumericsupplement"),
    ("enclosedcjk", "enclosedcjklettersandmonths"),
    ("enclosedideographicsup", "enclosedideographicsupplement"),
    ("ethiopicext", "ethiopicextended"),
    ("ethiopicexta", "ethiopicextendeda"),
    ("ethiopicextb", "ethiopicextendedb"),
    ("ethiopicsup", "ethiopicsupplement"),
    ("geometricshapesext", "geometricshapesextended"),
    ("georgianext", "georgianextended"),
    ("georgiansup", "georgiansupplement"),
    ("glagoliticsup", "glagoliticsupplement"),
    ("greek", "greekandcoptic"),
    ("greekext", "greekextended"),
    ("halfandfullforms", "halfwidthandfullwidthforms"),
    ("halfmarks", "combininghalfmarks"),
    ("hangul", "hangulsyllables"),
    ("highpusurrogates", "highprivateusesurrogates"),
    ("idc", "ideographicdescriptioncharacters"),
    ("ideographicsymbols", "ideographicsymbolsandpunctuation"),
    ("indicnumberforms", "commonindicnumberforms"),
    ("ipaext", "ipaextensions"),
    ("jamo", "hanguljamo"),
    ("jamoexta", "hanguljamoextendeda"),
    ("jamoextb", "hanguljamoextendedb"),
    ("kanaexta", "kanaextendeda"),
    ("kanaextb", "kanaextendedb"),
    ("kanasup", "kanasupplement"),
    ("kangxi", "kangxiradicals"),
    ("katakanaext", "katakanaphoneticextensions"),
    ("latin1sup", "latin1supplement"),
    ("latinexta", "latinextendeda"),
    ("latinextadditional", "latinextendedadditional"),
    ("latinextb", "latinextendedb"),
    ("latinextc", "latinextendedc"),
    ("latinextd", "latinextendedd"),
    ("latinexte", "latinextendede"),
    ("latinextf", "latinextendedf"),
    ("latinextg", "latinextendedg"),
    ("lisusup", "lisusupplement"),
    ("mahjong", "mahjongtiles"),
    ("mathalphanum", "mathematicalalphanumericsymbols"),
    ("mathoperators", "mathematicaloperators"),
    ("meeteimayekext", "meeteimayekextensions"),
    ("miscarrows", "miscellaneoussymbolsandarrows"),
    ("miscmathsymbolsa", "miscellaneousmathematicalsymbolsa"),
    ("miscmathsymbolsb", "miscellaneousmathematicalsymbolsb"),
    ("miscpictographs", "miscellaneoussymbolsandpictographs"),
    ("miscsymbols", "miscellaneoussymbols"),
    ("miscsymbolssup", "miscellaneoussymbolssupplement"),
    ("misctechnical", "miscellaneoustechnical"),
    ("modifierletters", "spacingmodifierletters"),
    ("mongoliansup", "mongoliansupplement"),
    ("music", "musicalsymbols"),
    ("myanmarexta", "myanmarextendeda"),
    ("myanmarextb", "myanmarextendedb"),
    ("myanmarextc", "myanmarextendedc"),
    ("nb", "noblock"),
    ("ocr", "opticalcharacterrecognition"),
    ("phaistos", "phaistosdisc"),
    ("phoneticext", "phoneticextensions"),
    ("phoneticextsup", "phoneticextensionssupplement"),
    ("pua", "privateusearea"),
    ("punctuation", "generalpunctuation"),
    ("rumi", "ruminumeralsymbols"),
    ("sharadasup", "sharadasupplement"),
    ("smallforms", "smallformvariants"),
    ("smallkanaext", "smallkanaextension"),
    ("sundanesesup", "sundanesesupplement"),
    ("suparrowsa", "supplementalarrowsa"),
    ("suparrowsb", "supplementalarrowsb"),
    ("suparrowsc", "supplementalarrowsc"),
    ("supmathoperators", "supplementalmathematicaloperators"),
    ("suppuaa", "supplementaryprivateuseareaa"),
    ("suppuab", "supplementaryprivateuseareab"),
    ("suppunctuation", "supplementalpunctuation"),
    (
        "supsymbolsandpictographs",
        "supplementalsymbolsandpictographs",
    ),
    ("superandsub", "superscriptsandsubscripts"),
    (
        "symbolsandpictographsexta",
        "symbolsandpictographsextendeda",
    ),
    (
        "symbolsforlegacycomputingsup",
        "symbolsforlegacycomputingsupplement",
    ),
    ("syriacsup", "syriacsupplement"),
    ("taixuanjing", "taixuanjingsymbols"),
    ("tamilsup", "tamilsupplement"),
    ("tangutcomponentssup", "tangutcomponentssupplement"),
    ("tangutsup", "tangutsupplement"),
    ("transportandmap", "transportandmapsymbols"),
    ("ucas", "unifiedcanadianaboriginalsyllabics"),
    ("ucasext", "unifiedcanadianaboriginalsyllabicsextended"),
    ("ucasexta", "unifiedcanadianaboriginalsyllabicsextendeda"),
    ("vedicext", "vedicextensions"),
    ("vs", "variationselectors"),
    ("vssup", "variationselectorssupplement"),
    ("yijing", "yijinghexagramsymbols"),
    ("znamennymusic", "znamennymusicalnotation"),
    // Additional aliases in the fourth UCD field.
    ("canadiansyllabics", "unifiedcanadianaboriginalsyllabics"),
    (
        "combiningmarksforsymbols",
        "combiningdiacriticalmarksforsymbols",
    ),
    ("cyrillicsupplementary", "cyrillicsupplement"),
    ("latin1", "latin1supplement"),
    ("privateuse", "privateusearea"),
];

fn indic_positional_category_ranges(value: &str) -> Option<Vec<i32>> {
    let canonical = match value {
        "na" | "notapplicable" => "na",
        "bottom" => "bottom",
        "bottomandleft" => "bottomandleft",
        "bottomandright" => "bottomandright",
        "left" => "left",
        "leftandright" => "leftandright",
        "overstruck" => "overstruck",
        "right" => "right",
        "top" => "top",
        "topandbottom" => "topandbottom",
        "topandbottomandleft" => "topandbottomandleft",
        "topandbottomandright" => "topandbottomandright",
        "topandleft" => "topandleft",
        "topandleftandright" => "topandleftandright",
        "topandright" => "topandright",
        "visualorderleft" => "visualorderleft",
        _ => return None,
    };
    INDIC_POSITIONAL_RANGES
        .get_or_init(|| {
            ranges_by_value(|code_point| {
                char::from_u32(code_point).map_or(
                    "na",
                    |character| match indic_positional_category(character) {
                        IndicPositionalCategory::NotApplicable => "na",
                        IndicPositionalCategory::Bottom => "bottom",
                        IndicPositionalCategory::BottomAndLeft => "bottomandleft",
                        IndicPositionalCategory::BottomAndRight => "bottomandright",
                        IndicPositionalCategory::Left => "left",
                        IndicPositionalCategory::LeftAndRight => "leftandright",
                        IndicPositionalCategory::Overstruck => "overstruck",
                        IndicPositionalCategory::Right => "right",
                        IndicPositionalCategory::Top => "top",
                        IndicPositionalCategory::TopAndBottom => "topandbottom",
                        IndicPositionalCategory::TopAndBottomAndLeft => "topandbottomandleft",
                        IndicPositionalCategory::TopAndBottomAndRight => "topandbottomandright",
                        IndicPositionalCategory::TopAndLeft => "topandleft",
                        IndicPositionalCategory::TopAndLeftAndRight => "topandleftandright",
                        IndicPositionalCategory::TopAndRight => "topandright",
                        IndicPositionalCategory::VisualOrderLeft => "visualorderleft",
                    },
                )
            })
        })
        .get(canonical)
        .cloned()
}

fn normalization_form(property: &str) -> Option<&'static str> {
    if matches_name(property, b"NFC_Quick_Check", b"nfc_qc") {
        Some("nfc")
    } else if matches_name(property, b"NFD_Quick_Check", b"nfd_qc") {
        Some("nfd")
    } else if matches_name(property, b"NFKC_Quick_Check", b"nfkc_qc") {
        Some("nfkc")
    } else if matches_name(property, b"NFKD_Quick_Check", b"nfkd_qc") {
        Some("nfkd")
    } else {
        None
    }
}

fn normalization_quick_check_ranges(form: &'static str, value: &str) -> Option<Vec<i32>> {
    let value = match value {
        "n" | "no" => 0,
        "m" | "maybe" if matches!(form, "nfc" | "nfkc") => 1,
        "y" | "yes" => 2,
        _ => return None,
    };
    NORMALIZATION_QUICK_CHECK_RANGES
        .get_or_init(|| {
            let mut nfc = ValueRangeBuilder::default();
            let mut nfd = ValueRangeBuilder::default();
            let mut nfkc = ValueRangeBuilder::default();
            let mut nfkd = ValueRangeBuilder::default();
            for code_point in 0..=u32::try_from(MAX_CODE_POINT).expect("valid code point") {
                let character = char::from_u32(code_point);
                nfc.push(
                    code_point,
                    character.map_or(2, |value| {
                        normalized_tag(quick_check_nfc(std::iter::once(value)))
                    }),
                );
                nfd.push(
                    code_point,
                    character.map_or(2, |value| {
                        normalized_tag(quick_check_nfd(std::iter::once(value)))
                    }),
                );
                nfkc.push(
                    code_point,
                    character.map_or(2, |value| {
                        normalized_tag(quick_check_nfkc(std::iter::once(value)))
                    }),
                );
                nfkd.push(
                    code_point,
                    character.map_or(2, |value| {
                        normalized_tag(quick_check_nfkd(std::iter::once(value)))
                    }),
                );
            }

            let mut ranges = HashMap::new();
            for (form, values) in [
                ("nfc", nfc.finish()),
                ("nfd", nfd.finish()),
                ("nfkc", nfkc.finish()),
                ("nfkd", nfkd.finish()),
            ] {
                for (value, value_ranges) in values {
                    ranges.insert((form, value), value_ranges);
                }
            }
            ranges
        })
        .get(&(form, value))
        .cloned()
}

const fn normalized_tag(value: IsNormalized) -> u8 {
    match value {
        IsNormalized::No => 0,
        IsNormalized::Maybe => 1,
        IsNormalized::Yes => 2,
    }
}

fn decomposition_ccc_ranges(value: &str, lead: bool) -> Option<Vec<i32>> {
    if value == "null" {
        return Some(Vec::new());
    }
    let combining_class = PropertyParser::<props::CanonicalCombiningClass>::new()
        .get_loose(value)?
        .to_icu4c_value();
    let (lead_ranges, trail_ranges) = DECOMPOSITION_CCC_RANGES.get_or_init(|| {
        let mut lead = ValueRangeBuilder::default();
        let mut trail = ValueRangeBuilder::default();
        for code_point in 0..=u32::try_from(MAX_CODE_POINT).expect("valid code point") {
            let (lead_class, trail_class) =
                char::from_u32(code_point).map_or((0, 0), |character| {
                    let mut decomposition = nfd(std::iter::once(character));
                    let first = decomposition
                        .next()
                        .expect("one scalar has a nonempty decomposition");
                    let last = decomposition.last().unwrap_or(first);
                    (
                        canonical_combining_class(first),
                        canonical_combining_class(last),
                    )
                });
            lead.push(code_point, lead_class);
            trail.push(code_point, trail_class);
        }
        (lead.finish(), trail.finish())
    });
    Some(
        if lead { lead_ranges } else { trail_ranges }
            .get(&combining_class)
            .cloned()
            .unwrap_or_default(),
    )
}

const DECOMPOSITION_TYPE_DATA: &[u8] = include_bytes!("unicode_decomposition.bin");
const DECOMPOSITION_TYPE_MAGIC: &[u8; 8] = b"ANTLRDT1";
const DECOMPOSITION_TYPE_SOURCE_HASH: &[u8; 64] =
    b"f44e5ceaf40edc1fe06ea0404e8bebc7d356dcc38aac076543b6874008a06e3e";
const DECOMPOSITION_TYPE_HEADER_LENGTH: usize = 8 + 3 + 64 + 4;
const DECOMPOSITION_TYPE_RECORD_LENGTH: usize = 9;

fn decomposition_type_ranges(value: &str) -> Option<Vec<i32>> {
    let decomposition_type = match value {
        "none" => 0,
        "can" | "canonical" => 1,
        "com" | "compat" => 2,
        "enc" | "circle" => 3,
        "fin" | "final" => 4,
        "font" => 5,
        "fra" | "fraction" => 6,
        "init" | "initial" => 7,
        "iso" | "isolated" => 8,
        "med" | "medial" => 9,
        "nar" | "narrow" => 10,
        "nb" | "nobreak" => 11,
        "sml" | "small" => 12,
        "sqr" | "square" => 13,
        "sub" => 14,
        "sup" | "super" => 15,
        "vert" | "vertical" => 16,
        "wide" => 17,
        _ => return None,
    };
    Some(
        DECOMPOSITION_TYPE_RANGES
            .get_or_init(build_decomposition_type_ranges)
            .get(&decomposition_type)
            .cloned()
            .unwrap_or_default(),
    )
}

fn build_decomposition_type_ranges() -> RangesByU8 {
    let mut ranges = decode_compatibility_decomposition_ranges();
    let canonical = ranges_matching(|code_point| {
        char::from_u32(code_point).is_some_and(|character| {
            let mut decomposition = nfd(std::iter::once(character));
            decomposition.next() != Some(character) || decomposition.next().is_some()
        })
    });

    let mut decomposed = canonical.clone();
    for decomposition_type in 2..=17 {
        decomposed = union_ranges(
            &decomposed,
            ranges.get(&decomposition_type).map_or(&[], Vec::as_slice),
        );
    }
    ranges.insert(0, subtract_ranges(&[0, MAX_CODE_POINT], &decomposed));
    ranges.insert(1, canonical);
    ranges
}

fn decode_compatibility_decomposition_ranges() -> RangesByU8 {
    let data = DECOMPOSITION_TYPE_DATA;
    assert_eq!(
        data.get(..DECOMPOSITION_TYPE_MAGIC.len()),
        Some(DECOMPOSITION_TYPE_MAGIC.as_slice()),
        "invalid Unicode decomposition data magic"
    );
    assert_eq!(
        data.get(8..11),
        Some(&[17, 0, 0][..]),
        "Unicode decomposition data must be version 17.0.0"
    );
    assert_eq!(
        data.get(11..75),
        Some(DECOMPOSITION_TYPE_SOURCE_HASH.as_slice()),
        "Unicode decomposition data source hash differs"
    );

    let record_count = usize::try_from(u32::from_be_bytes(
        data.get(75..79)
            .expect("Unicode decomposition data has a record count")
            .try_into()
            .expect("record count occupies four bytes"),
    ))
    .expect("Unicode decomposition record count fits usize");
    assert_eq!(
        data.len(),
        DECOMPOSITION_TYPE_HEADER_LENGTH + record_count * DECOMPOSITION_TYPE_RECORD_LENGTH,
        "Unicode decomposition data length differs from its record count"
    );

    let mut ranges = RangesByU8::new();
    for record in
        data[DECOMPOSITION_TYPE_HEADER_LENGTH..].chunks_exact(DECOMPOSITION_TYPE_RECORD_LENGTH)
    {
        let decomposition_type = record[0];
        assert!(
            (2..=17).contains(&decomposition_type),
            "invalid compatibility decomposition type {decomposition_type}"
        );
        let start = u32::from_be_bytes(
            record[1..5]
                .try_into()
                .expect("range start occupies four bytes"),
        );
        let stop = u32::from_be_bytes(
            record[5..9]
                .try_into()
                .expect("range stop occupies four bytes"),
        );
        assert!(
            start <= stop
                && stop <= u32::try_from(MAX_CODE_POINT).expect("maximum code point fits u32"),
            "invalid Unicode decomposition range {start:x}..{stop:x}"
        );
        let start = i32::try_from(start).expect("Unicode range start fits i32");
        let stop = i32::try_from(stop).expect("Unicode range stop fits i32");

        let property_ranges = ranges.entry(decomposition_type).or_default();
        assert!(
            property_ranges
                .last()
                .is_none_or(|previous_stop| *previous_stop < start),
            "Unicode decomposition ranges are not sorted"
        );
        property_ranges.extend([start, stop]);
    }
    ranges
}

#[derive(Debug)]
struct ValueRangeBuilder<Key> {
    current: Option<(Key, u32)>,
    ranges: HashMap<Key, Vec<i32>>,
}

impl<Key> Default for ValueRangeBuilder<Key> {
    fn default() -> Self {
        Self {
            current: None,
            ranges: HashMap::new(),
        }
    }
}

impl<Key> ValueRangeBuilder<Key>
where
    Key: Eq + Hash,
{
    fn push(&mut self, code_point: u32, value: Key) {
        let Some((current, start)) = self.current.take() else {
            self.current = Some((value, code_point));
            return;
        };
        if current == value {
            self.current = Some((current, start));
        } else {
            self.ranges.entry(current).or_default().extend([
                i32::try_from(start).expect("Unicode range start fits i32"),
                i32::try_from(code_point - 1).expect("Unicode range end fits i32"),
            ]);
            self.current = Some((value, code_point));
        }
    }

    fn finish(mut self) -> HashMap<Key, Vec<i32>> {
        if let Some((current, start)) = self.current {
            self.ranges.entry(current).or_default().extend([
                i32::try_from(start).expect("Unicode range start fits i32"),
                MAX_CODE_POINT,
            ]);
        }
        self.ranges
    }
}

fn ranges_by_value<Key>(mut value_at: impl FnMut(u32) -> Key) -> HashMap<Key, Vec<i32>>
where
    Key: Eq + Hash,
{
    let mut ranges = ValueRangeBuilder::default();
    for code_point in 0..=u32::try_from(MAX_CODE_POINT).expect("valid code point") {
        ranges.push(code_point, value_at(code_point));
    }
    ranges.finish()
}

fn ranges_matching(mut predicate: impl FnMut(u32) -> bool) -> Vec<i32> {
    let mut result = Vec::new();
    let mut start = None;
    for code_point in 0..=u32::try_from(MAX_CODE_POINT).expect("valid code point") {
        match (start, predicate(code_point)) {
            (None, true) => start = Some(code_point),
            (Some(range_start), false) => {
                result.extend([
                    i32::try_from(range_start).expect("Unicode range start fits i32"),
                    i32::try_from(code_point - 1).expect("Unicode range end fits i32"),
                ]);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(range_start) = start {
        result.extend([
            i32::try_from(range_start).expect("Unicode range start fits i32"),
            MAX_CODE_POINT,
        ]);
    }
    result
}

fn ranges_from_set(set: CodePointSetDataBorrowed<'_>) -> Vec<i32> {
    ranges_from_iter(set.iter_ranges())
}

fn ranges_from_iter(ranges: impl Iterator<Item = std::ops::RangeInclusive<u32>>) -> Vec<i32> {
    let mut result = Vec::new();
    for range in ranges {
        let (start, stop) = range.into_inner();
        result.push(i32::try_from(start).expect("Unicode range start fits i32"));
        result.push(i32::try_from(stop).expect("Unicode range end fits i32"));
    }
    result
}

fn subtract_ranges(include: &[i32], exclude: &[i32]) -> Vec<i32> {
    let mut result = Vec::new();
    let mut exclude_index = 0;
    for included in include.chunks_exact(2) {
        let mut next = included[0];
        let stop = included[1];
        while exclude_index < exclude.len() && exclude[exclude_index + 1] < next {
            exclude_index += 2;
        }
        let mut index = exclude_index;
        while index < exclude.len() && exclude[index] <= stop {
            if next < exclude[index] {
                result.extend([next, exclude[index] - 1]);
            }
            next = next.max(exclude[index + 1].saturating_add(1));
            if next > stop {
                break;
            }
            index += 2;
        }
        if next <= stop {
            result.extend([next, stop]);
        }
    }
    result
}

fn union_ranges(left: &[i32], right: &[i32]) -> Vec<i32> {
    let mut pairs = left
        .chunks_exact(2)
        .chain(right.chunks_exact(2))
        .map(|range| (range[0], range[1]))
        .collect::<Vec<_>>();
    pairs.sort_unstable();

    let mut result: Vec<i32> = Vec::with_capacity(pairs.len() * 2);
    for (start, stop) in pairs {
        if result
            .last()
            .is_some_and(|previous_stop| start <= previous_stop.saturating_add(1))
        {
            let previous_stop = result.last_mut().expect("range has a previous stop");
            *previous_stop = (*previous_stop).max(stop);
        } else {
            result.extend([start, stop]);
        }
    }
    result
}

fn matches_name(name: &str, long: &[u8], short: &[u8]) -> bool {
    normalized_bytes(long).eq(name.bytes()) || normalized_bytes(short).eq(name.bytes())
}

fn normalized_bytes(name: &[u8]) -> impl Iterator<Item = u8> + '_ {
    name.iter()
        .copied()
        .filter(|byte| !matches!(byte, b'-' | b'_' | b' ' | b'\t' | b'\n' | b'\r'))
        .map(|byte| byte.to_ascii_lowercase())
}

fn normalize_property_name(name: &str) -> String {
    normalized_bytes(name.as_bytes()).map(char::from).collect()
}

#[cfg(test)]
#[path = "unicode_icu_tests.rs"]
mod icu_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn contains(property: &str, code_point: i32) -> bool {
        property_ranges(property)
            .expect("known Unicode property")
            .chunks_exact(2)
            .any(|range| (range[0]..=range[1]).contains(&code_point))
    }

    #[test]
    fn resolves_properties_aliases_and_normalization() {
        assert_eq!(property_ranges("Gothic"), Some(&[66352, 66378][..]));
        assert_eq!(property_ranges("Deseret"), Some(&[66560, 66639][..]));
        assert_eq!(
            property_ranges("InLatin-Extended-B"),
            property_ranges("block=latin_extended_b"),
        );
        assert!(property_ranges("not_a_unicode_property").is_none());
    }

    #[test]
    fn uses_pinned_java_simple_case_mappings() {
        assert_eq!(simple_lowercase(i32::from(b'A')), i32::from(b'a'));
        assert_eq!(simple_uppercase(i32::from(b'a')), i32::from(b'A'));
        assert_eq!(simple_uppercase(0x00df), 0x00df);
        assert_eq!(simple_lowercase(0x0130), i32::from(b'i'));
    }

    #[test]
    fn unicode_general_categories_latin_matches_java() {
        assert!(contains("Lu", i32::from(b'X')));
        assert!(!contains("Lu", i32::from(b'x')));
        assert!(contains("Ll", i32::from(b'x')));
        assert!(!contains("Ll", i32::from(b'X')));
        assert!(contains("L", i32::from(b'X')));
        assert!(contains("L", i32::from(b'x')));
        assert!(contains("N", i32::from(b'0')));
        assert!(contains("Z", i32::from(b' ')));
    }

    #[test]
    fn unicode_general_categories_bmp_matches_java() {
        assert!(contains("Lu", 0x1e3a));
        assert!(!contains("Lu", 0x1e3b));
        assert!(contains("Ll", 0x1e3b));
        assert!(!contains("Ll", 0x1e3a));
        assert!(contains("L", 0x1e3a));
        assert!(contains("L", 0x1e3b));
        assert!(contains("N", 0x1bb0));
        assert!(!contains("N", 0x1e3a));
        assert!(contains("Z", 0x2028));
        assert!(!contains("Z", 0x1e3a));
    }

    #[test]
    fn unicode_general_categories_smp_matches_java() {
        assert!(contains("Lu", 0x1d5d4));
        assert!(!contains("Lu", 0x1d770));
        assert!(contains("Ll", 0x1d770));
        assert!(!contains("Ll", 0x1d5d4));
        assert!(contains("L", 0x1d5d4));
        assert!(contains("L", 0x1d770));
        assert!(contains("N", 0x11c50));
        assert!(!contains("N", 0x1d5d4));
    }

    #[test]
    fn unicode_category_aliases_match_java() {
        assert!(contains("Lowercase_Letter", i32::from(b'x')));
        assert!(!contains("Lowercase_Letter", i32::from(b'X')));
        assert!(contains("Letter", i32::from(b'x')));
        assert!(!contains("Letter", i32::from(b'0')));
        assert!(contains("Enclosing_Mark", 0x20e2));
        assert!(!contains("Enclosing_Mark", i32::from(b'x')));
    }

    #[test]
    fn unicode_binary_properties_match_java() {
        assert!(contains("Emoji", 0x1f4a9));
        assert!(!contains("Emoji", i32::from(b'X')));
        assert!(contains("alnum", i32::from(b'9')));
        assert!(!contains("alnum", 0x1f4a9));
        assert!(contains("Dash", i32::from(b'-')));
        assert!(contains("Hex", i32::from(b'D')));
        assert!(!contains("Hex", i32::from(b'Q')));
    }

    #[test]
    fn unicode_binary_property_aliases_match_java() {
        assert!(contains("Ideo", 0x611b));
        assert!(!contains("Ideo", i32::from(b'X')));
        assert!(contains("Soft_Dotted", 0x0456));
        assert!(!contains("Soft_Dotted", i32::from(b'X')));
        assert!(contains("Noncharacter_Code_Point", 0xffff));
        assert!(!contains("Noncharacter_Code_Point", i32::from(b'X')));
    }

    #[test]
    fn unicode_scripts_match_java() {
        assert!(contains("Zyyy", i32::from(b'0')));
        assert!(contains("Latn", i32::from(b'X')));
        assert!(contains("Hani", 0x4e04));
        assert!(contains("Cyrl", 0x0404));
    }

    #[test]
    fn unicode_script_equals_matches_java() {
        assert!(contains("Script=Zyyy", i32::from(b'0')));
        assert!(contains("Script=Latn", i32::from(b'X')));
        assert!(contains("Script=Hani", 0x4e04));
        assert!(contains("Script=Cyrl", 0x0404));
    }

    #[test]
    fn unicode_script_aliases_match_java() {
        assert!(contains("Common", i32::from(b'0')));
        assert!(contains("Latin", i32::from(b'X')));
        assert!(contains("Han", 0x4e04));
        assert!(contains("Cyrillic", 0x0404));
    }

    #[test]
    fn unicode_blocks_match_java() {
        assert!(contains("InASCII", i32::from(b'0')));
        assert!(contains("InCJK", 0x4e04));
        assert!(contains("InCyrillic", 0x0404));
        assert!(contains("InMisc_Pictographs", 0x1f4a9));
    }

    #[test]
    fn unicode_block_equals_matches_java() {
        assert!(contains("Block=ASCII", i32::from(b'0')));
        assert!(contains("Block=CJK", 0x4e04));
        assert!(contains("Block=Cyrillic", 0x0404));
        assert!(contains("Block=Misc_Pictographs", 0x1f4a9));
    }

    #[test]
    fn unicode_block_aliases_match_java() {
        assert!(contains("InBasic_Latin", i32::from(b'0')));
        assert!(contains("InMiscellaneous_Mathematical_Symbols_B", 0x29be));
    }

    #[test]
    fn enumerated_property_equals_matches_java() {
        assert!(!contains("Grapheme_Cluster_Break=E_Base", 0x1f47e));
        assert!(!contains("Grapheme_Cluster_Break=E_Base", 0x1038));
        assert!(contains("East_Asian_Width=Ambiguous", 0x00a1));
        assert!(!contains("East_Asian_Width=Ambiguous", 0x00a2));
    }

    #[test]
    fn extended_pictographic_matches_java() {
        assert!(contains("Extended_Pictographic", 0x1f588));
        assert!(!contains("Extended_Pictographic", i32::from(b'0')));
    }

    #[test]
    fn emoji_presentation_matches_java() {
        assert!(contains("EmojiPresentation=EmojiDefault", 0x1f4a9));
        assert!(!contains("EmojiPresentation=EmojiDefault", i32::from(b'0')));
        assert!(!contains("EmojiPresentation=EmojiDefault", i32::from(b'A')));
        assert!(!contains("EmojiPresentation=TextDefault", 0x1f4a9));
        assert!(contains("EmojiPresentation=TextDefault", i32::from(b'0')));
        assert!(!contains("EmojiPresentation=TextDefault", i32::from(b'A')));
    }

    #[test]
    fn property_case_insensitivity_matches_java() {
        assert!(contains("l", i32::from(b'x')));
        assert!(!contains("l", i32::from(b'0')));
        assert!(contains("common", i32::from(b'0')));
        assert!(contains("Alnum", i32::from(b'0')));
    }

    #[test]
    fn property_dash_same_as_underscore_matches_java() {
        assert!(contains("InLatin-1", 0x00f0));
    }

    #[test]
    fn modifying_unicode_data_should_throw_matches_java() {
        fn requires_read_only_static_slice(_: &'static [i32]) {}

        requires_read_only_static_slice(property_ranges("L").expect("known Unicode property"));
    }
}
