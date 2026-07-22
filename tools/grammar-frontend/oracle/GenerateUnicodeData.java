import com.ibm.icu.lang.UCharacter;
import com.ibm.icu.util.VersionInfo;
import org.antlr.v4.runtime.misc.Interval;
import org.antlr.v4.runtime.misc.IntervalSet;
import org.antlr.v4.unicode.UnicodeDataTemplateController;
import org.stringtemplate.v4.ST;
import org.stringtemplate.v4.STGroup;
import org.stringtemplate.v4.STGroupString;

import javax.tools.JavaCompiler;
import javax.tools.ToolProvider;
import java.io.BufferedWriter;
import java.io.InputStream;
import java.lang.reflect.Field;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.util.HexFormat;
import java.util.Map;
import java.util.TreeMap;

/**
 * Regenerates ANTLR's {@code UnicodeData} class with the ICU4J version supplied
 * on the classpath, compiles it as a fixture-only classpath overlay, and writes
 * a compact digest oracle for every property and alias.
 */
public final class GenerateUnicodeData {
    private static final String TEMPLATE_RESOURCE =
            "org/antlr/v4/tool/templates/unicodedata.st";

    private GenerateUnicodeData() {}

    public static void main(String[] args) throws Exception {
        if (args.length != 3) {
            System.err.println(
                    "usage: GenerateUnicodeData <generated-source-root> "
                            + "<overlay-class-root> <property-oracle>");
            System.exit(2);
        }

        Path sourceRoot = Path.of(args[0]);
        Path classRoot = Path.of(args[1]);
        Path propertyOracle = Path.of(args[2]);
        Path sourcePath =
                sourceRoot.resolve("org/antlr/v4/unicode/UnicodeData.java");
        Files.createDirectories(sourcePath.getParent());
        Files.createDirectories(classRoot);

        String templateSource;
        ClassLoader loader = UnicodeDataTemplateController.class.getClassLoader();
        try (InputStream input = loader.getResourceAsStream(TEMPLATE_RESOURCE)) {
            if (input == null) {
                throw new IllegalStateException(
                        "ANTLR jar does not contain " + TEMPLATE_RESOURCE);
            }
            templateSource = new String(input.readAllBytes(), StandardCharsets.UTF_8);
        }

        STGroup group = new STGroupString(TEMPLATE_RESOURCE, templateSource);
        ST template = group.getInstanceOf("unicodedata");
        if (template == null) {
            throw new IllegalStateException("ANTLR Unicode template has no unicodedata entry");
        }
        for (Map.Entry<String, Object> property :
                UnicodeDataTemplateController.getProperties().entrySet()) {
            template.add(property.getKey(), property.getValue());
        }
        Files.writeString(sourcePath, template.render(), StandardCharsets.UTF_8);

        JavaCompiler compiler = ToolProvider.getSystemJavaCompiler();
        if (compiler == null) {
            throw new IllegalStateException("a full JDK is required to build the Unicode overlay");
        }
        int status =
                compiler.run(
                        null,
                        System.out,
                        System.err,
                        "-classpath",
                        System.getProperty("java.class.path"),
                        "-d",
                        classRoot.toString(),
                        sourcePath.toString());
        if (status != 0) {
            throw new IllegalStateException("UnicodeData compilation failed with exit " + status);
        }

        writePropertyOracle(classRoot, propertyOracle);
        System.out.println("icu4j_version\t" + VersionInfo.ICU_VERSION);
        System.out.println("unicode_version\t" + UCharacter.getUnicodeVersion());
    }

    private static void writePropertyOracle(Path classRoot, Path outputPath) throws Exception {
        URL[] urls = {classRoot.toUri().toURL()};
        try (URLClassLoader loader =
                        new URLClassLoader(urls, GenerateUnicodeData.class.getClassLoader()) {
                            @Override
                            protected Class<?> loadClass(String name, boolean resolve)
                                    throws ClassNotFoundException {
                                if (!name.equals("org.antlr.v4.unicode.UnicodeData")) {
                                    return super.loadClass(name, resolve);
                                }
                                synchronized (getClassLoadingLock(name)) {
                                    Class<?> loaded = findLoadedClass(name);
                                    if (loaded == null) {
                                        loaded = findClass(name);
                                    }
                                    if (resolve) {
                                        resolveClass(loaded);
                                    }
                                    return loaded;
                                }
                            }
                        }) {
            Class<?> unicodeData =
                    Class.forName("org.antlr.v4.unicode.UnicodeData", true, loader);
            Map<String, IntervalSet> properties =
                    readMap(unicodeData, "propertyCodePointRanges");
            Map<String, String> aliases = readMap(unicodeData, "propertyAliases");

            Files.createDirectories(outputPath.getParent());
            try (BufferedWriter output =
                    Files.newBufferedWriter(outputPath, StandardCharsets.UTF_8)) {
                output.write("# ANTLR 4.13.2 Unicode property oracle; Unicode 17.0\n");
                for (Map.Entry<String, IntervalSet> property :
                        new TreeMap<>(properties).entrySet()) {
                    output.write("property\t");
                    output.write(property.getKey());
                    output.write('\t');
                    output.write(Integer.toString(property.getValue().getIntervals().size()));
                    output.write('\t');
                    output.write(intervalDigest(property.getValue()));
                    output.write('\n');
                }
                for (Map.Entry<String, String> alias : new TreeMap<>(aliases).entrySet()) {
                    output.write("alias\t");
                    output.write(alias.getKey());
                    output.write('\t');
                    output.write(alias.getValue());
                    output.write('\n');
                }
            }
        }
    }

    @SuppressWarnings("unchecked")
    private static <T> Map<String, T> readMap(Class<?> owner, String fieldName)
            throws ReflectiveOperationException {
        Field field = owner.getDeclaredField(fieldName);
        field.setAccessible(true);
        return (Map<String, T>) field.get(null);
    }

    private static String intervalDigest(IntervalSet ranges) throws Exception {
        MessageDigest digest = MessageDigest.getInstance("SHA-256");
        ByteBuffer encoded = ByteBuffer.allocate(Integer.BYTES * 2);
        for (Interval range : ranges.getIntervals()) {
            encoded.clear();
            encoded.putInt(range.a);
            encoded.putInt(range.b);
            digest.update(encoded.array());
        }
        return HexFormat.of().formatHex(digest.digest());
    }
}
