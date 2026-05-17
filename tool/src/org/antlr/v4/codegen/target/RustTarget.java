/*
 * Clean-room ANTLR Rust target integration.
 *
 * This file intentionally contains only target-specific naming, escaping, and
 * reserved-word policy. Code generation behavior belongs in Rust.stg templates.
 */
package org.antlr.v4.codegen.target;

import org.antlr.v4.codegen.CodeGenerator;
import org.antlr.v4.codegen.Target;
import org.antlr.v4.parse.ANTLRParser;
import org.antlr.v4.tool.Grammar;
import org.antlr.v4.tool.ast.GrammarAST;
import org.stringtemplate.v4.ST;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashSet;
import java.util.List;
import java.util.Locale;
import java.util.Set;

public class RustTarget extends Target {
    private static final String[] RUST_KEYWORDS = {
            "as", "async", "await", "break", "const", "continue", "crate",
            "dyn", "else", "enum", "extern", "false", "fn", "for", "gen",
            "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut",
            "pub", "ref", "return", "Self", "self", "static", "struct",
            "super", "trait", "true", "type", "unsafe", "use", "where",
            "while", "abstract", "become", "box", "do", "final", "macro",
            "override", "priv", "try", "typeof", "unsized", "virtual",
            "yield", "_"
    };

    private final Set<String> badWords = new HashSet<>();

    public RustTarget(CodeGenerator gen) {
        super(gen, "Rust");
    }

    @Override
    public Set<String> getBadWords() {
        if (badWords.isEmpty()) {
            badWords.addAll(Arrays.asList(RUST_KEYWORDS));
            badWords.add("recog");
            badWords.add("input");
            badWords.add("ctx");
        }
        return badWords;
    }

    @Override
    protected boolean visibleGrammarSymbolCausesIssueInGeneratedCode(GrammarAST idNode) {
        return getBadWords().contains(idNode.getText());
    }

    @Override
    public String encodeIntAsCharEscape(int value) {
        if (value < 0
                || value > Character.MAX_CODE_POINT
                || (value >= Character.MIN_SURROGATE && value <= Character.MAX_SURROGATE)) {
            throw new IllegalArgumentException("invalid Unicode scalar value: " + value);
        }

        switch (value) {
            case '\n':
                return "\\n";
            case '\r':
                return "\\r";
            case '\t':
                return "\\t";
            case '\\':
                return "\\\\";
            case '\'':
                return "\\'";
            default:
                break;
        }

        if (value >= 0x20 && value <= 0x7e) {
            return Character.toString((char) value);
        }
        return String.format(Locale.ROOT, "\\u{%x}", value);
    }

    @Override
    public String getRecognizerFileName(boolean header) {
        Grammar grammar = getCodeGenerator().g;
        String stem;
        switch (grammar.getType()) {
            case ANTLRParser.LEXER:
                stem = stripSuffix(grammar.name, "Lexer") + "Lexer";
                break;
            case ANTLRParser.PARSER:
                stem = stripSuffix(grammar.name, "Parser") + "Parser";
                break;
            case ANTLRParser.COMBINED:
                stem = grammar.name + "Parser";
                break;
            default:
                stem = grammar.name;
                break;
        }
        return rustModuleName(stem) + codeFileExtension();
    }

    @Override
    public String getListenerFileName(boolean header) {
        return rustModuleName(getCodeGenerator().g.name + "Listener") + codeFileExtension();
    }

    @Override
    public String getVisitorFileName(boolean header) {
        return rustModuleName(getCodeGenerator().g.name + "Visitor") + codeFileExtension();
    }

    @Override
    public String getBaseListenerFileName(boolean header) {
        return rustModuleName(getCodeGenerator().g.name + "BaseListener") + codeFileExtension();
    }

    @Override
    public String getBaseVisitorFileName(boolean header) {
        return rustModuleName(getCodeGenerator().g.name + "BaseVisitor") + codeFileExtension();
    }

    private String codeFileExtension() {
        ST extension = getTemplates().getInstanceOf("codeFileExtension");
        return extension.render();
    }

    private static String stripSuffix(String value, String suffix) {
        return value.endsWith(suffix) ? value.substring(0, value.length() - suffix.length()) : value;
    }

    private static String rustModuleName(String value) {
        return String.join("_", splitIdentifierWords(value));
    }

    private static List<String> splitIdentifierWords(String value) {
        List<String> words = new ArrayList<>();
        StringBuilder out = new StringBuilder();
        for (int i = 0; i < value.length(); i++) {
            char ch = value.charAt(i);
            if (!Character.isLetterOrDigit(ch)) {
                if (out.length() > 0) {
                    words.add(out.toString().toLowerCase(Locale.ROOT));
                    out.setLength(0);
                }
                continue;
            }

            Character previous = i > 0 ? value.charAt(i - 1) : null;
            Character next = i + 1 < value.length() ? value.charAt(i + 1) : null;
            boolean startsNewWord = out.length() > 0
                    && Character.isUpperCase(ch)
                    && ((previous != null && (Character.isLowerCase(previous) || Character.isDigit(previous)))
                    || (previous != null && Character.isUpperCase(previous)
                    && next != null && Character.isLowerCase(next)));
            if (startsNewWord) {
                words.add(out.toString().toLowerCase(Locale.ROOT));
                out.setLength(0);
            }
            out.append(ch);
        }
        if (out.length() > 0) {
            words.add(out.toString().toLowerCase(Locale.ROOT));
        }
        return words;
    }
}
