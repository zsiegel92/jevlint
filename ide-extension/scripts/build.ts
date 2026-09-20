import { context } from "esbuild";

const build = await context({
	entryPoints: ["src/extension.ts"],
	bundle: true,
	platform: "node",
	format: "cjs",
	target: "node20",
	external: ["vscode"],
	outfile: "dist/extension.cjs",
	sourcemap: true,
	sourcesContent: false,
	logLevel: "info",
});

if (process.argv.includes("--watch")) {
	await build.watch();
	console.log("Watching JevLint extension sources...");
} else {
	await build.rebuild();
	await build.dispose();
}
