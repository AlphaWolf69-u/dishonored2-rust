// Ghidra headless post-script. Exports pseudocode in address-ordered chunks.
// Arguments: output-directory [maximum-functions; 0 means all]
// The output is local research data; do not redistribute game decompilation.

import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.data.Composite;
import ghidra.program.model.data.DataType;
import ghidra.program.model.data.DataTypeComponent;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionIterator;
import java.io.BufferedWriter;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Iterator;
import java.util.Set;

public class ExportDecomp extends GhidraScript {
    private static final int FUNCTIONS_PER_CHUNK = 100;

    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 1) {
            throw new IllegalArgumentException("Expected output directory");
        }
        Path output = Path.of(args[0]);
        Files.createDirectories(output);
        Path statusFile = output.resolve("status.txt");
        Files.writeString(statusFile, "RUNNING: Ghidra pseudocode export\n", StandardCharsets.UTF_8);
        int maxFunctions = args.length > 1 ? Integer.parseInt(args[1]) : 0;
        if (maxFunctions < 0) {
            throw new IllegalArgumentException("maximum-functions must be nonnegative");
        }

        DecompInterface decompiler = new DecompInterface();
        decompiler.toggleCCode(true);
        decompiler.toggleSyntaxTree(false);
        if (!decompiler.openProgram(currentProgram)) {
            throw new IOException("Cannot open program in decompiler");
        }

        int total = 0;
        int success = 0;
        int failed = 0;
        BufferedWriter chunk = null;
        try (BufferedWriter manifest = Files.newBufferedWriter(
                output.resolve("functions.tsv"), StandardCharsets.UTF_8);
             BufferedWriter calls = Files.newBufferedWriter(
                output.resolve("calls.tsv"), StandardCharsets.UTF_8)) {
            manifest.write("address\tname\tnamespace\tbytes\tstatus\tchunk\n");
            calls.write("caller\tcallee\n");
            FunctionIterator functions = currentProgram.getFunctionManager().getFunctions(true);
            while (functions.hasNext() && !monitor.isCancelled()
                    && (maxFunctions == 0 || total < maxFunctions)) {
                Function function = functions.next();
                if (total % FUNCTIONS_PER_CHUNK == 0) {
                    if (chunk != null) {
                        chunk.close();
                    }
                    String chunkName = String.format("chunk_%06d.c", total / FUNCTIONS_PER_CHUNK);
                    chunk = Files.newBufferedWriter(output.resolve(chunkName), StandardCharsets.UTF_8);
                }
                String chunkName = String.format("chunk_%06d.c", total / FUNCTIONS_PER_CHUNK);
                String address = function.getEntryPoint().toString();
                String name = clean(function.getName());
                String namespace = clean(function.getParentNamespace().getName(true));
                String status = "ok";
                String code = "";
                try {
                    DecompileResults result = decompiler.decompileFunction(function, 60, monitor);
                    if (result.decompileCompleted() && result.getDecompiledFunction() != null) {
                        code = result.getDecompiledFunction().getC();
                        success++;
                    } else {
                        status = "failed: " + clean(result.getErrorMessage());
                        failed++;
                    }
                } catch (Exception error) {
                    status = "exception: " + clean(error.toString());
                    failed++;
                }
                chunk.write("\n/* GHIDRA_FUNCTION " + address + " " + name + " */\n");
                if (status.equals("ok")) {
                    chunk.write(code);
                    chunk.write("\n");
                } else {
                    chunk.write("/* " + status + " */\n");
                }
                manifest.write(address + "\t" + name + "\t" + namespace + "\t"
                    + function.getBody().getNumAddresses() + "\t" + status + "\t" + chunkName + "\n");
                Set<Function> callees = function.getCalledFunctions(monitor);
                for (Function callee : callees) {
                    calls.write(address + "\t" + callee.getEntryPoint() + "\n");
                }
                total++;
                if (total % 1000 == 0) {
                    manifest.flush();
                    calls.flush();
                    chunk.flush();
                    println("Exported " + total + " functions; decompiled=" + success
                        + " failed=" + failed);
                }
            }
        } finally {
            if (chunk != null) {
                chunk.close();
            }
            decompiler.dispose();
        }
        int types = 0;
        int fields = 0;
        try (BufferedWriter typeWriter = Files.newBufferedWriter(
                 output.resolve("types.tsv"), StandardCharsets.UTF_8);
             BufferedWriter fieldWriter = Files.newBufferedWriter(
                 output.resolve("type_fields.tsv"), StandardCharsets.UTF_8)) {
            typeWriter.write("category\tname\tkind\tbytes\n");
            fieldWriter.write("category\tparent\tordinal\toffset\tbytes\tfield\ttype\n");
            Iterator<DataType> iterator = currentProgram.getDataTypeManager().getAllDataTypes();
            while (iterator.hasNext() && !monitor.isCancelled()) {
                DataType type = iterator.next();
                String category = clean(type.getCategoryPath().getPath());
                String name = clean(type.getName());
                typeWriter.write(category + "\t" + name + "\t"
                    + type.getClass().getSimpleName() + "\t" + type.getLength() + "\n");
                types++;
                if (type instanceof Composite) {
                    for (DataTypeComponent component : ((Composite) type).getComponents()) {
                        fieldWriter.write(category + "\t" + name + "\t"
                            + component.getOrdinal() + "\t" + component.getOffset() + "\t"
                            + component.getLength() + "\t" + clean(component.getFieldName())
                            + "\t" + clean(component.getDataType().getDisplayName()) + "\n");
                        fields++;
                    }
                }
            }
        }
        String summary = "COMPLETE: functions=" + total
            + " decompiled=" + success + " failed=" + failed
            + " data-types=" + types + " fields=" + fields;
        Files.writeString(statusFile, summary + "\n", StandardCharsets.UTF_8);
        println("Decompilation export " + summary);
    }

    private static String clean(String value) {
        if (value == null) {
            return "";
        }
        return value.replace('\t', ' ').replace('\n', ' ').replace('\r', ' ');
    }
}
