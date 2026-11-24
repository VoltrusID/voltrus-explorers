// esbuild bundling. `--production` minifies; `--integration` builds the
// @vscode/test-electron runner and suite into dist-test/.
import * as esbuild from "esbuild";

const args = new Set(process.argv.slice(2));
const production = args.has("--production");

const common = {
  bundle: true,
  platform: "node",
  format: "cjs",
  target: "node20",
  external: ["vscode"],
  sourcemap: !production,
  minify: production,
  logLevel: "info",
};

if (args.has("--integration")) {
  await esbuild.build({
    ...common,
    entryPoints: {
      runTest: "test/integration/runTest.ts",
      "suite/index": "test/integration/suite/index.ts",
      "suite/extension.test": "test/integration/suite/extension.test.ts",
    },
    outdir: "dist-test",
    external: ["vscode", "mocha", "@vscode/test-electron"],
  });
} else {
  const ctx = await esbuild.context({
    ...common,
    entryPoints: ["src/extension.ts"],
    outfile: "dist/extension.js",
  });
  if (args.has("--watch")) {
    await ctx.watch();
  } else {
    await ctx.rebuild();
    await ctx.dispose();
  }
}
