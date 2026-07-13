import java.io.PrintWriter;
import java.nio.file.Path;
import java.util.List;

import org.antlr.v4.runtime.CharStreams;
import org.antlr.v4.runtime.CommonTokenStream;
import org.antlr.v4.runtime.ParserRuleContext;
import org.antlr.v4.runtime.Token;
import org.antlr.v4.runtime.tree.ErrorNode;
import org.antlr.v4.runtime.tree.ParseTree;
import org.antlr.v4.runtime.tree.TerminalNode;

public final class TypeScriptParityDumper {
    private TypeScriptParityDumper() {}

    private static String rustDebugString(String value) {
        StringBuilder out = new StringBuilder("\"");
        for (int index = 0; index < value.length(); index++) {
            char ch = value.charAt(index);
            switch (ch) {
                case '\\' -> out.append("\\\\");
                case '"' -> out.append("\\\"");
                case '\n' -> out.append("\\n");
                case '\r' -> out.append("\\r");
                case '\t' -> out.append("\\t");
                case '\0' -> out.append("\\0");
                default -> {
                    if (ch < 0x20 || ch == 0x7f) {
                        out.append("\\u{").append(Integer.toHexString(ch)).append('}');
                    } else {
                        out.append(ch);
                    }
                }
            }
        }
        return out.append('"').toString();
    }

    private static void dumpTree(
            ParseTree tree, TypeScriptParser parser, PrintWriter out, int depth) {
        String pad = "  ".repeat(depth);
        if (tree instanceof ErrorNode) {
            out.println(pad + "Err(" + rustDebugString(tree.getText()) + ")");
            return;
        }
        if (tree instanceof TerminalNode) {
            out.println(pad + "Term(" + rustDebugString(tree.getText()) + ")");
            return;
        }
        ParserRuleContext rule = (ParserRuleContext) tree;
        String name = parser.getRuleNames()[rule.getRuleIndex()];
        out.println(pad + "Rule(" + name + ", children=" + rule.getChildCount() + ")");
        for (int index = 0; index < rule.getChildCount(); index++) {
            dumpTree(rule.getChild(index), parser, out, depth + 1);
        }
    }

    public static void main(String[] args) throws Exception {
        Path input = null;
        boolean tokensOnly = false;
        for (int index = 0; index < args.length; index++) {
            switch (args[index]) {
                case "--input" -> input = Path.of(args[++index]);
                case "--tokens" -> tokensOnly = true;
                default -> throw new IllegalArgumentException("unknown argument: " + args[index]);
            }
        }
        if (input == null) {
            throw new IllegalArgumentException("missing --input <path>");
        }

        TypeScriptLexer lexer = new TypeScriptLexer(CharStreams.fromPath(input));
        CommonTokenStream stream = new CommonTokenStream(lexer);
        stream.fill();
        PrintWriter out = new PrintWriter(System.out);
        if (tokensOnly) {
            List<Token> tokens = stream.getTokens();
            for (Token token : tokens) {
                if (token.getType() != Token.EOF) {
                    out.println(token.getType() + "\t" + token.getChannel() + "\t"
                            + rustDebugString(token.getText()));
                }
            }
            out.flush();
            return;
        }

        TypeScriptParser parser = new TypeScriptParser(stream);
        ParseTree tree = parser.program();
        if (parser.getNumberOfSyntaxErrors() != 0) {
            throw new IllegalStateException(
                    "parse produced " + parser.getNumberOfSyntaxErrors() + " syntax error(s)");
        }
        dumpTree(tree, parser, out, 0);
        out.flush();
    }
}
