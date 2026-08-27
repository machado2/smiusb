// Decompile selected functions from a locally installed SMI binary.
// @category SMIUSB

import java.io.File;
import java.io.PrintWriter;
import java.util.regex.Pattern;

import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionIterator;

public class DecompileSmi extends GhidraScript {
    @Override
    protected void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length != 2) {
            throw new IllegalArgumentException(
                "usage: DecompileSmi.java OUTPUT_FILE FUNCTION_REGEX");
        }

        File output = new File(args[0]);
        Pattern wanted = Pattern.compile(args[1]);
        DecompInterface decompiler = new DecompInterface();
        decompiler.toggleCCode(true);
        decompiler.toggleSyntaxTree(true);
        decompiler.setSimplificationStyle("decompile");
        if (!decompiler.openProgram(currentProgram)) {
            throw new IllegalStateException(decompiler.getLastMessage());
        }

        int matched = 0;
        try (PrintWriter writer = new PrintWriter(output, "UTF-8")) {
            FunctionIterator functions =
                currentProgram.getFunctionManager().getFunctions(true);
            while (functions.hasNext() && !monitor.isCancelled()) {
                Function function = functions.next();
                String qualifiedName = function.getName(true);
                if (!wanted.matcher(qualifiedName).find()) {
                    continue;
                }

                matched++;
                writer.printf("\n/* ===== %s @ %s ===== */\n",
                    qualifiedName, function.getEntryPoint());
                DecompileResults result =
                    decompiler.decompileFunction(function, 120, monitor);
                if (result.decompileCompleted()) {
                    writer.println(
                        result.getDecompiledFunction().getC());
                } else {
                    writer.printf("/* decompilation failed: %s */\n",
                        result.getErrorMessage());
                }
            }
            writer.printf("\n/* matched functions: %d */\n", matched);
        } finally {
            decompiler.dispose();
        }

        println("Decompiled " + matched + " functions into " + output);
    }
}
