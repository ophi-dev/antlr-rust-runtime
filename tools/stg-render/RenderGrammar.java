import org.stringtemplate.v4.ST;
import org.stringtemplate.v4.STGroup;
import org.stringtemplate.v4.STGroupFile;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;

/**
 * Renders one ANTLR runtime-testsuite descriptor grammar through a target
 * {@code .test.stg} template group, mirroring the upstream harness
 * ({@code RuntimeTests.prepareGrammars}): {@code new ST(group, grammar).render()}.
 *
 * <p>Run with the Java single-file source launcher against the ANTLR complete
 * jar (which bundles StringTemplate 4):
 *
 * <pre>java -cp antlr-4.13.2-complete.jar RenderGrammar.java Rust.test.stg In.g4 Out.g4</pre>
 */
public final class RenderGrammar {
    private RenderGrammar() {}

    public static void main(String[] args) throws Exception {
        if (args.length != 3) {
            System.err.println("usage: RenderGrammar <group.stg> <grammar-template> <rendered-grammar>");
            System.exit(2);
        }
        STGroup group = new STGroupFile(args[0]);
        String grammar = Files.readString(Path.of(args[1]), StandardCharsets.UTF_8);
        ST st = new ST(group, grammar);
        Files.writeString(Path.of(args[2]), st.render(), StandardCharsets.UTF_8);
    }
}
